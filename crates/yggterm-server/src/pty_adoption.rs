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
//! ## Adoption is Linux-only, and says so in the type system
//!
//! Everything that makes an adopted child *knowable* — the `/proc/<pid>/stat`
//! identity, the task-count liveness probe, the received-master ioctls — is
//! Linux machinery. So the `Adopted` variant, [`ReceivedMasterPty`], and the
//! `/proc` readers are all `#[cfg(target_os = "linux")]`: on any other target
//! adoption cannot be *expressed*, let alone answered wrongly.
//!
//! That gate is on the adoption half rather than on the whole module for one
//! reason: `terminal.rs` holds `Arc<Mutex<PtyChildHandle>>` unconditionally, and
//! gating the module would force a second, non-Linux encoding of the same
//! concept — the one thing this crate's rules forbid outright. `Owned` needs no
//! `/proc` and compiles everywhere, so one type still owns the concept on every
//! target.
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
//!
//! ## …and the fourth, which cost this module a review round
//!
//! 4. **Identity is not liveness, and only identity may gate a signal.** The
//!    first draft gated `kill`/`signal` on the same predicate as `is_running`,
//!    so anything that read as "not running" also became UNKILLABLE and
//!    `shutdown()` returned `Ok` over it. A process whose thread-group leader
//!    called `pthread_exit` reads state `Z` in `/proc/<pid>/stat` while running
//!    normally in another thread, so that was not hypothetical. The two
//!    questions are now two functions: [`adopted_identity_matches`] (existence
//!    plus start time — the ONLY gate on signalling) and
//!    [`adopted_process_is_alive`] (identity plus a thread group that still has
//!    a live task).

use std::io::Result as IoResult;

use portable_pty::{Child, ExitStatus};

#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::io::{Error as IoError, Read, Write};
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::sync::Mutex;

#[cfg(target_os = "linux")]
use portable_pty::{MasterPty, PtySize};

// ---------------------------------------------------------------------------
// /proc identity — Linux only, because that is the only place it exists
// ---------------------------------------------------------------------------

/// Read `/proc/<pid>/stat` field 22 (`starttime`), the clock ticks after boot
/// at which the process started.
///
/// Parsed from the LAST `)` because field 2 is the executable name in
/// parentheses and may itself contain spaces and parentheses — splitting the
/// whole line on whitespace is the classic way to read the wrong field.
#[cfg(target_os = "linux")]
pub(crate) fn process_start_time(pid: u32) -> Option<u64> {
    proc_stat(pid).map(|(_state, start_time)| start_time)
}

/// `(state, starttime)` from `/proc/<pid>/stat` — fields 3 and 22.
///
/// Parsed from the LAST `)` because field 2 is the executable name in
/// parentheses and may itself contain spaces and parentheses; splitting the
/// whole line on whitespace is the classic way to read the wrong field.
#[cfg(target_os = "linux")]
fn proc_stat(pid: u32) -> Option<(char, u64)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit_once(')')?.1;
    // The remainder begins at field 3 (state), so field N is at index N - 3.
    let mut fields = after_comm.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let start_time = fields.nth(22 - 3 - 1)?.parse().ok()?;
    Some((state, start_time))
}

/// How many tasks (kernel threads) the thread group still has.
///
/// `/proc/<pid>/task` lists one entry per task, including the leader. A REAL
/// zombie has exactly one — its own already-dead leader. A process whose leader
/// exited while siblings kept running has more, and that is the only thing that
/// tells the two apart from outside.
#[cfg(target_os = "linux")]
fn live_task_count(pid: u32) -> usize {
    match std::fs::read_dir(format!("/proc/{pid}/task")) {
        Ok(entries) => entries.flatten().count(),
        Err(_) => 0,
    }
}

/// IDENTITY ONLY: does `pid` still exist, and is it still the process we
/// adopted?
///
/// **This deliberately does not ask whether the process is running**, and it is
/// the ONLY gate on `kill`/`signal`. Gating a signal on liveness sounds
/// conservative and is the opposite: any process the liveness probe misreads as
/// dead becomes permanently unkillable, and every teardown path then reports
/// success over something still alive. The question a signal must ask is "is
/// this still the same process", never "is it still working".
///
/// A start time that no longer matches means the pid was recycled: the original
/// is gone and something unrelated now answers to its number. That — and only
/// that — must refuse the signal.
#[cfg(target_os = "linux")]
fn adopted_identity_matches(pid: u32, start_time: u64) -> bool {
    matches!(proc_stat(pid), Some((_state, current)) if current == start_time)
}

/// LIVENESS: the same identity, plus a thread group that still has a live task.
///
/// Two things make this more than a `/proc` existence check:
///
/// - A **zombie** is not running. It matters here in a way it never did for an
///   owned child: `try_wait` reaps and the entry disappears, but nothing reaps
///   an adopted one on our behalf, so between its death and init collecting it
///   `/proc/<pid>` still exists and the start time still matches. Reporting that
///   as alive would make every shutdown path wait out its full timeout on an
///   already-dead process.
/// - A **leader-exited process is not a zombie**, even though `/proc/<pid>/stat`
///   says `Z`. When a thread-group leader calls `pthread_exit` the group keeps
///   running in its other threads while the leader task lingers as a zombie, and
///   the state field reports the LEADER. The task count separates the cases: a
///   real zombie is down to its leader alone.
#[cfg(target_os = "linux")]
fn adopted_process_is_alive(pid: u32, start_time: u64) -> bool {
    match proc_stat(pid) {
        Some((state, current)) if current == start_time => state != 'Z' || live_task_count(pid) > 1,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// PtyChildHandle
// ---------------------------------------------------------------------------

/// The child behind a PTY session — one we spawned, or one we inherited by
/// receiving its master fd.
pub(crate) enum PtyChildHandle {
    /// We spawned it. `waitpid` works, so exit status is available.
    Owned(Box<dyn Child + Send + Sync>),
    /// We received its master fd from another process, which then exited. The
    /// process re-parented to init: it is not our child, `waitpid` would fail,
    /// and `/proc` plus the start time are the only identity we have.
    #[cfg(target_os = "linux")]
    Adopted { pid: u32, start_time: u64 },
}

impl PtyChildHandle {
    pub(crate) fn owned(child: Box<dyn Child + Send + Sync>) -> Self {
        PtyChildHandle::Owned(child)
    }

    /// Adopt `pid`, capturing its start time now. Returns `None` if the process
    /// is already gone — adopting a pid we cannot pin an identity to would
    /// create exactly the stranger-killing hazard this type exists to prevent.
    #[cfg(target_os = "linux")]
    pub(crate) fn adopt(pid: u32) -> Option<Self> {
        process_start_time(pid).map(|start_time| PtyChildHandle::Adopted { pid, start_time })
    }

    /// Adopt a pid whose start time the SENDER already read.
    ///
    /// The spike proved the triple `(fd, pid, start_time)` must travel together:
    /// re-reading the start time on the receiving side races the very pid reuse
    /// it is meant to detect.
    #[cfg(target_os = "linux")]
    pub(crate) fn adopt_with_start_time(pid: u32, start_time: u64) -> Self {
        PtyChildHandle::Adopted { pid, start_time }
    }

    pub(crate) fn is_adopted(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            matches!(self, PtyChildHandle::Adopted { .. })
        }
        // Adoption has no representation off Linux, so the answer is not a
        // guess — the variant does not exist to be held.
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    pub(crate) fn process_id(&self) -> Option<u32> {
        match self {
            PtyChildHandle::Owned(child) => child.process_id(),
            #[cfg(target_os = "linux")]
            PtyChildHandle::Adopted { pid, .. } => Some(*pid),
        }
    }

    /// Is the child still running?
    ///
    /// This replaces `try_wait().is_none()` at every call site ON PURPOSE. The
    /// old shape forced callers to think in terms of an exit status, which an
    /// adopted child can never supply; every site that actually asked "is it
    /// running" now asks that directly and works for both variants.
    ///
    /// **It returns a `Result` and not a `bool` on purpose too.** An owned
    /// child's probe is `waitpid`, which really can fail, and folding that
    /// failure into `false` means "the probe broke" and "it exited" become the
    /// same answer — which is how a teardown ends up tracing success over a live
    /// process. Callers that genuinely cannot act on an error say so at their
    /// own call site.
    pub(crate) fn is_running(&mut self) -> IoResult<bool> {
        match self {
            PtyChildHandle::Owned(child) => child.try_wait().map(|status| status.is_none()),
            #[cfg(target_os = "linux")]
            PtyChildHandle::Adopted { pid, start_time } => {
                Ok(adopted_process_is_alive(*pid, *start_time))
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
            #[cfg(target_os = "linux")]
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
            #[cfg(target_os = "linux")]
            PtyChildHandle::Adopted { pid, start_time } => {
                // IDENTITY, not liveness. Signalling a recycled pid is the one
                // catastrophic failure mode of adoption, and refusing to signal
                // anything the liveness probe dislikes is the other one.
                if !adopted_identity_matches(*pid, *start_time) {
                    return Ok(());
                }
                // SAFETY: the pid was just confirmed to still be the process we
                // adopted, by start time as well as by number.
                unsafe { libc::kill(*pid as libc::pid_t, libc::SIGKILL) };
                Ok(())
            }
        }
    }

    /// Send a signal, honouring the same identity check as [`kill`](Self::kill).
    ///
    /// Returns whether the signal was DELIVERED — `false` means "refused,
    /// because this pid is no longer the process we adopted", never "the
    /// process looked idle".
    #[cfg(unix)]
    pub(crate) fn signal(&mut self, signal: libc::c_int) -> bool {
        let Some(pid) = self.process_id() else {
            return false;
        };
        #[cfg(target_os = "linux")]
        if let PtyChildHandle::Adopted { start_time, .. } = self
            && !adopted_identity_matches(pid, *start_time)
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
            #[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
#[allow(dead_code)]
pub(crate) struct ReceivedMasterPty {
    fd: OwnedFd,
    /// The shell's pid and start time, carried because they must travel with
    /// the fd (see [`PtyChildHandle::adopt_with_start_time`]).
    pid: u32,
    start_time: u64,
    /// `MasterPty::take_writer` is documented as invalid to call twice.
    took_writer: Mutex<bool>,
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
impl ReceivedMasterPty {
    /// Build a master from the triple the spike proved must travel together.
    pub(crate) fn new(fd: OwnedFd, pid: u32, start_time: u64) -> Self {
        ReceivedMasterPty {
            fd,
            pid,
            start_time,
            took_writer: Mutex::new(false),
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

    /// Duplicate the master fd **with `FD_CLOEXEC` set**.
    ///
    /// `F_DUPFD_CLOEXEC`, never plain `dup(2)`: `dup` produces a descriptor with
    /// the close-on-exec flag CLEARED regardless of the original, so every
    /// process the daemon spawns afterwards would inherit a live copy of this
    /// PTY master. A leaked master is not a tidiness problem — it holds the pty
    /// open, so the slave's hangup never arrives, the shell never sees EOF, and
    /// the session cannot end. This matches what `portable_pty` does through
    /// `filedescriptor::FileDescriptor::try_clone`, which is `F_DUPFD_CLOEXEC`
    /// for exactly this reason.
    fn dup_file(&self) -> IoResult<File> {
        let duped = unsafe { libc::fcntl(self.fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
        if duped < 0 {
            return Err(IoError::last_os_error());
        }
        // SAFETY: `fcntl(F_DUPFD_CLOEXEC)` just returned an owned descriptor.
        Ok(unsafe { File::from_raw_fd(duped) })
    }
}

/// The read half of an adopted master.
///
/// Maps `EIO` to `Ok(0)`, matching `portable_pty` 0.9.0's `PtyFd`:
///
/// > EIO indicates that the slave pty has been closed. Treat this as EOF so
/// > that `std::io::Read::read_to_string` and similar functions gracefully
/// > terminate when they encounter this condition
///
/// Without the mapping, an adopted session ending would surface as an io error
/// where an owned one surfaces as end-of-stream — i.e. the reader loop would
/// paint an error line into the user's viewport at every normal exit.
#[cfg(target_os = "linux")]
struct AdoptedPtyReader(File);

#[cfg(target_os = "linux")]
impl Read for AdoptedPtyReader {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        match self.0.read(buf) {
            Err(ref error) if error.raw_os_error() == Some(libc::EIO) => Ok(0),
            other => other,
        }
    }
}

/// The write half of an adopted master.
///
/// `MasterPty::take_writer` is documented as "dropping the writer will send EOF
/// to the slave end", and `portable_pty` 0.9.0's `UnixMasterWriter` implements
/// that by reading the termios `VEOF` character and writing a newline followed
/// by it — EOF is only interpreted at the start of a line, so the newline is
/// part of the contract, not decoration. An adopted master that closed silently
/// would leave the shell waiting for input that is never coming.
#[cfg(target_os = "linux")]
struct AdoptedPtyWriter(File);

#[cfg(target_os = "linux")]
impl Write for AdoptedPtyWriter {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> IoResult<()> {
        self.0.flush()
    }
}

#[cfg(target_os = "linux")]
impl Drop for AdoptedPtyWriter {
    fn drop(&mut self) {
        // SAFETY: `self.0` is an open pty master fd for the lifetime of `self`.
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(self.0.as_raw_fd(), &mut termios) } == 0 {
            let eot = termios.c_cc[libc::VEOF];
            if eot != 0 {
                let _ = self.0.write_all(&[b'\n', eot]);
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl MasterPty for ReceivedMasterPty {
    fn resize(&self, size: PtySize) -> Result<(), anyhow::Error> {
        // `libc::winsize` and `libc::TIOCSWINSZ`, never a local `#[repr(C)]`
        // copy and a hardcoded 0x5414: a second encoding of a kernel ABI is a
        // second thing that can drift, and the hardcoded constant was already
        // wrong for every non-Linux target.
        let win = libc::winsize {
            ws_row: size.rows,
            ws_col: size.cols,
            ws_xpixel: size.pixel_width,
            ws_ypixel: size.pixel_height,
        };
        let rc = unsafe { libc::ioctl(self.fd.as_raw_fd(), libc::TIOCSWINSZ, &win) };
        if rc != 0 {
            return Err(anyhow::anyhow!(
                "TIOCSWINSZ on adopted pty failed: {}",
                IoError::last_os_error()
            ));
        }
        Ok(())
    }

    fn get_size(&self) -> Result<PtySize, anyhow::Error> {
        let mut win: libc::winsize = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::ioctl(self.fd.as_raw_fd(), libc::TIOCGWINSZ, &mut win) };
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
        Ok(Box::new(AdoptedPtyReader(self.dup_file()?)))
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
        Ok(Box::new(AdoptedPtyWriter(writer)))
    }

    fn process_group_leader(&self) -> Option<libc::pid_t> {
        let pgid = unsafe { libc::tcgetpgrp(self.fd.as_raw_fd()) };
        (pgid > 0).then_some(pgid)
    }

    fn as_raw_fd(&self) -> Option<RawFd> {
        Some(self.fd.as_raw_fd())
    }

    fn tty_name(&self) -> Option<PathBuf> {
        // The trait requires the method but permits `None`, and NOTHING in this
        // crate reads it. The previous body called `ptsname_r`, which does not
        // exist on Apple targets and needed a `c_char` buffer whose element type
        // differs between x86_64 and aarch64 — a portability hazard bought for a
        // value with no reader. If a reader ever appears, resolve the name where
        // it is needed, not speculatively here.
        None
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use portable_pty::{CommandBuilder, native_pty_system};
    use std::time::{Duration, Instant};

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
        assert!(handle.is_running().expect("a /proc probe cannot fail"));
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
        assert!(
            !gone.is_running().expect("a /proc probe cannot fail"),
            "a dead adopted child is not running",
        );
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
        assert!(
            genuine.is_running().expect("a /proc probe cannot fail"),
            "the real identity must match",
        );

        // Same pid, a start time we never saw — i.e. the number was reused.
        let mut impostor = PtyChildHandle::adopt_with_start_time(me, real.wrapping_add(1));
        assert!(
            !impostor.is_running().expect("a /proc probe cannot fail"),
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
        assert!(
            adopted.is_running().expect("a /proc probe cannot fail"),
            "the sleeper should be running",
        );

        adopted.kill().expect("kill the adopted child");

        let deadline = Instant::now() + Duration::from_secs(5);
        while adopted.is_running().unwrap_or(false) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !adopted.is_running().expect("a /proc probe cannot fail"),
            "an adopted child must actually die when killed — dropping the master fd only \
             SIGHUPs the foreground group, so this is the ONLY thing that ends it",
        );

        // This test is the one place the process really is our child, so reap it
        // rather than leaking a zombie into the rest of the suite.
        let _ = spawned.wait();
    }

    /// The zombie window, which only exists for adopted children: we are not
    /// the reaper, so between death and collection `/proc/<pid>` still exists
    /// and the start time still matches.
    ///
    /// **Unconditional on purpose.** The first version of this lock wrapped its
    /// assertions in `if saw_zombie`, which passes vacuously on any run that
    /// misses the window — the failure mode a lock exists to prevent. Nothing
    /// but this test may reap the child, so the window cannot legitimately be
    /// missed, and a run that misses it is a fact worth failing over.
    #[test]
    fn a_zombie_is_not_alive() {
        let mut spawned = std::process::Command::new("true")
            .spawn()
            .expect("spawn a process that exits immediately");
        let pid = spawned.id();
        let before = process_start_time(pid).expect("start time readable while alive");

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut state = ' ';
        while Instant::now() < deadline {
            match proc_stat(pid) {
                Some((observed, _)) => {
                    state = observed;
                    if observed == 'Z' {
                        break;
                    }
                }
                None => panic!(
                    "the /proc entry for {pid} vanished — nothing but this test may reap it, so \
                     the zombie window cannot be missed",
                ),
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(state, 'Z', "the zombie window must be observable");

        let after = process_start_time(pid).expect("a zombie still has a /proc/<pid>/stat");
        assert_eq!(
            before, after,
            "the start time STILL matches a dead process — identity alone cannot detect death, \
             which is exactly why liveness reads the state field as well",
        );
        assert_eq!(
            live_task_count(pid),
            1,
            "a REAL zombie is down to its own dead leader; that is what separates it from a \
             live process whose leader exited",
        );

        let mut handle = PtyChildHandle::adopt_with_start_time(pid, after);
        assert!(
            !handle.is_running().expect("a /proc probe cannot fail"),
            "a zombie still has a /proc entry and a matching start time, but it is NOT running",
        );

        // …and being dead must NOT make it unsignallable: the teardown path
        // signals first and asks later, so a refusal here would turn every
        // already-exited session into an error on the way out.
        assert!(
            handle.signal(0),
            "signalling a zombie is a no-op at the kernel, not a refusal — the identity still \
             matches, and only identity may gate a signal",
        );
        assert!(
            handle.kill().is_ok(),
            "killing an already-dead adopted child must not error out the teardown path",
        );

        let _ = spawned.wait();
    }

    // -----------------------------------------------------------------------
    // Identity vs liveness: the review's falsification, made permanent
    // -----------------------------------------------------------------------

    /// Environment variable that turns [`leader_exit_helper_subprocess`] from an
    /// inert ignored test into the helper process.
    const LEADER_EXIT_HELPER_ENV: &str = "YGGTERM_PTY_ADOPTION_LEADER_EXIT_HELPER";

    /// Exit THIS task and no other.
    ///
    /// `SYS_exit` terminates the calling task; `exit()` compiles to `exit_group`
    /// and would take the whole process with it, leaving nothing to observe.
    /// A raw syscall is async-signal-safe, which is why the handler may do it.
    extern "C" fn exit_this_task_only(_signal: libc::c_int) {
        unsafe { libc::syscall(libc::SYS_exit, 0) };
    }

    /// NOT A TEST — the helper process for
    /// [`a_live_process_whose_leader_exited_is_still_killable`].
    ///
    /// It is `#[ignore]`d and additionally inert unless
    /// [`LEADER_EXIT_HELPER_ENV`] is set, so `--include-ignored` cannot hang a
    /// normal run. Re-executing this test binary is what makes the shape
    /// reproducible with no C compiler and no external helper binary: the
    /// process ends up with a live task and a dead thread-group leader.
    #[test]
    #[ignore = "helper process for a_live_process_whose_leader_exited_is_still_killable"]
    fn leader_exit_helper_subprocess() {
        if std::env::var_os(LEADER_EXIT_HELPER_ENV).is_none() {
            return;
        }
        // A task that outlives the leader, so the thread group stays alive.
        std::thread::spawn(|| std::thread::sleep(Duration::from_secs(300)));

        let handler: extern "C" fn(libc::c_int) = exit_this_task_only;
        unsafe { libc::signal(libc::SIGUSR1, handler as libc::sighandler_t) };

        // `tgkill(tgid, tid, sig)` with tid == tgid targets the LEADER task
        // specifically, wherever libtest happens to be running this function.
        let pid = std::process::id() as libc::pid_t;
        unsafe { libc::syscall(libc::SYS_tgkill, pid, pid, libc::SIGUSR1) };

        std::thread::sleep(Duration::from_secs(300));
    }

    /// Consequence 4, locked: a LIVE process whose thread-group leader exited
    /// reads state `Z`, and it must still be killable.
    ///
    /// This is the review's falsification, kept as a permanent lock. With
    /// `kill`/`signal` gated on the liveness predicate the measured result was
    ///
    /// > `tasks=2 kill(pid,0)_says_alive=true is_running()=false
    /// > signal(0)_fired=false kill()_ok=true process_still_present_after_kill()=true`
    ///
    /// — i.e. `shutdown()` returned `Ok` over a process it had refused to
    /// signal. Each of those four lies is asserted against below.
    #[test]
    fn a_live_process_whose_leader_exited_is_still_killable() {
        let exe = std::env::current_exe().expect("the test binary's own path");
        let mut helper = std::process::Command::new(exe)
            .arg("pty_adoption::tests::leader_exit_helper_subprocess")
            .arg("--exact")
            .arg("--ignored")
            .env(LEADER_EXIT_HELPER_ENV, "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("re-exec this test binary as the leader-exit helper");
        let pid = helper.id();

        // Wait for the leader task to die while the group keeps running.
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut state = ' ';
        let mut tasks = 0;
        while Instant::now() < deadline {
            if let Some((observed, _)) = proc_stat(pid) {
                state = observed;
                tasks = live_task_count(pid);
                if observed == 'Z' && tasks > 1 {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        // Preconditions. A failure here is a real failure, not a skip: without
        // them the assertions below would prove nothing.
        assert_eq!(
            state, 'Z',
            "precondition: the helper's leader task must be a zombie (tasks={tasks})",
        );
        assert!(
            tasks > 1,
            "precondition: a sibling task must still be running (tasks={tasks})",
        );
        // Independent of this module: the kernel says the process exists.
        assert_eq!(
            unsafe { libc::kill(pid as libc::pid_t, 0) },
            0,
            "precondition: the kernel accepts a signal for this pid, so it is alive",
        );

        let start_time = process_start_time(pid).expect("the helper is still in /proc");
        let mut handle = PtyChildHandle::adopt_with_start_time(pid, start_time);

        let running = handle.is_running().expect("a /proc probe cannot fail");
        let signalled = handle.signal(0);
        let killed = handle.kill();

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut exited = false;
        while Instant::now() < deadline {
            if helper
                .try_wait()
                .expect("waiting on our own helper")
                .is_some()
            {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !exited {
            // Never leave the helper behind, whatever the assertions say.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            let _ = helper.wait();
        }

        // Every assertion carries the whole measurement, in the review's own
        // shape, so whichever one fires the report shows the full picture
        // rather than one bit of it.
        let measured = format!(
            "tasks={tasks} is_running()={running} signal(0)_fired={signalled} \
             kill()_ok={} process_exited_after_kill()={exited}",
            killed.is_ok(),
        );

        assert!(
            exited,
            "kill() must actually END a process whose leader already exited. Gating the signal \
             on the liveness predicate made it permanently unkillable while shutdown() returned \
             Ok over it — {measured}",
        );
        assert!(
            signalled,
            "signal() must fire: only identity may gate a signal, and the identity matches — \
             {measured}",
        );
        assert!(killed.is_ok(), "kill() must not error — {measured}");
        assert!(
            running,
            "a process with {tasks} tasks is RUNNING even though its leader reads 'Z': the state \
             field describes the LEADER, not the thread group — {measured}",
        );
    }

    // -----------------------------------------------------------------------
    // The OWNED path — the variant every production site still uses
    // -----------------------------------------------------------------------

    /// Spawn a real child on a real PTY, exactly as `PtySessionRuntime` does.
    ///
    /// **The pty pair comes back with it and the caller MUST hold it alive.**
    /// Dropping the master closes the pty, which sends `SIGHUP` to the
    /// foreground process group and ends the child by itself — so a lock that
    /// let the pair drop would pass even when `kill()` did nothing whatsoever.
    /// That is not hypothetical: the first version of the two locks below did
    /// exactly that and read GREEN under both of the mutations they exist for.
    fn spawn_owned_child_on_a_pty(seconds: &str) -> (portable_pty::PtyPair, PtyChildHandle, u32) {
        let pair = native_pty_system()
            .openpty(PtySize::default())
            .expect("open a pty pair");
        let mut command = CommandBuilder::new("sleep");
        command.arg(seconds);
        let child = pair
            .slave
            .spawn_command(command)
            .expect("spawn a sleeper on the pty");
        let pid = child
            .process_id()
            .expect("a freshly spawned child has a pid");
        (pair, PtyChildHandle::owned(child), pid)
    }

    /// `kill` on an OWNED child has to end it. The adopted path had a lock for
    /// this from the start; the owned path did not, and a no-op `kill` there
    /// left the whole suite green.
    #[test]
    fn killing_an_owned_child_actually_ends_it() {
        // `_pair` keeps the pty open for the whole test: if the master closed,
        // the resulting SIGHUP would end the child and this would pass without
        // kill() doing anything.
        let (_pair, mut handle, pid) = spawn_owned_child_on_a_pty("30");
        assert!(
            handle.is_running().expect("try_wait on our own child"),
            "the sleeper should be running",
        );

        handle.kill().expect("kill the owned child");

        let deadline = Instant::now() + Duration::from_secs(5);
        while handle.is_running().unwrap_or(false) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let still_running = handle.is_running().expect("try_wait on our own child");

        // Clean up before asserting, so a failure cannot leak a 30s sleeper.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        handle.wait_for_exit();

        assert!(
            !still_running,
            "an owned child must actually die when killed — every shutdown path in \
             terminal.rs relies on kill() ending the session it was told to end",
        );
    }

    /// `wait_for_exit` on an OWNED child has to WAIT, and waiting means
    /// `waitpid` — which also reaps. A `wait_for_exit` that returns immediately
    /// lets the caller trace a shutdown as finished while the process is still
    /// running.
    ///
    /// **Nothing kills the child here, deliberately.** `portable_pty`'s
    /// `ChildKiller for std::process::Child` calls `try_wait()` inside `kill()`
    /// as part of its SIGHUP-then-SIGKILL escalation, so `kill()` REAPS. A
    /// version of this test that killed first could not tell whether
    /// `wait_for_exit` had done anything at all — measured: it passed with the
    /// whole owned arm replaced by `{}`. So the child is left to exit on its
    /// own and `wait_for_exit` is the only thing that can be waiting for it.
    #[test]
    fn wait_for_exit_waits_for_and_reaps_an_owned_child() {
        // `_pair` again: without it the SIGHUP from closing the master would
        // end the child early, before the wait is under test.
        let (_pair, mut handle, pid) = spawn_owned_child_on_a_pty("1");

        let started = Instant::now();
        handle.wait_for_exit();
        let waited = started.elapsed();

        // This check has to happen BEFORE anything else touches the handle:
        // `is_running()` would itself reap through `try_wait` and hide the bug.
        let lingering = proc_stat(pid);
        assert!(
            lingering.is_none(),
            "wait_for_exit must block until the child is gone and reaped; /proc/{pid} still \
             reports {lingering:?} the instant it returned, so the caller was told the shutdown \
             had finished over a process still in the table",
        );
        assert!(
            waited >= Duration::from_millis(500),
            "wait_for_exit returned after {waited:?} for a child that had ~1s left to live — it \
             did not wait for anything",
        );
    }

    /// A child whose liveness probe FAILS. `waitpid` really can fail — `ECHILD`
    /// if something else reaped the pid first — and this is the only way to make
    /// it fail on demand.
    #[derive(Debug)]
    struct ProbeFailsChild;

    impl portable_pty::ChildKiller for ProbeFailsChild {
        fn kill(&mut self) -> IoResult<()> {
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
            Box::new(ProbeFailsChild)
        }
    }

    impl Child for ProbeFailsChild {
        fn try_wait(&mut self) -> IoResult<Option<ExitStatus>> {
            Err(IoError::other("the liveness probe itself failed"))
        }

        fn wait(&mut self) -> IoResult<ExitStatus> {
            Err(IoError::other("the liveness probe itself failed"))
        }

        fn process_id(&self) -> Option<u32> {
            None
        }
    }

    /// "The probe broke" and "it exited" must not be the same answer.
    ///
    /// The first draft of this module returned `bool` and mapped `Err` to
    /// `false`, so a failed `waitpid` read as *exited* — which made the graceful
    /// shutdown loop trace `graceful_shutdown_completed` over a process it knew
    /// nothing about. That is the teardown-lies class at the child layer, so the
    /// error has to survive the call.
    #[test]
    fn a_failed_liveness_probe_is_an_error_not_an_exit() {
        let mut handle = PtyChildHandle::owned(Box::new(ProbeFailsChild));
        let probed = handle.is_running();
        assert!(
            probed.is_err(),
            "a failed probe must be reported as a failure, not silently as \
             `not running`; got {probed:?}",
        );
    }

    /// …and the one teardown path that cannot propagate must still TRACE it.
    ///
    /// **Honest limitation: source-text scan**, same as the tripwire below. The
    /// graceful-shutdown loop runs on a detached thread inside a
    /// `PtySessionRuntime` that no test can construct with an injected child, so
    /// the distinct trace is asserted on the source instead of the behaviour.
    /// The *behavioural* half — that the error reaches that call site at all —
    /// is [`a_failed_liveness_probe_is_an_error_not_an_exit`] above.
    #[test]
    fn the_graceful_shutdown_loop_traces_a_failed_probe_distinctly() {
        let terminal_src = include_str!("terminal.rs");
        assert!(
            terminal_src.contains("graceful_shutdown_probe_failed"),
            "the graceful shutdown loop must keep a DISTINCT trace for a failed liveness \
             probe — folding it into graceful_shutdown_completed reports a successful \
             teardown over a process nobody probed successfully",
        );
        let loop_body = terminal_src
            .split_once("fn shutdown_with_force_after")
            .expect("the graceful shutdown path must still exist")
            .1;
        let completed = loop_body
            .find("graceful_shutdown_completed")
            .expect("the success trace must exist");
        let failed = loop_body
            .find("graceful_shutdown_probe_failed")
            .expect("the probe-failure trace must be in the same loop");
        assert!(
            failed > completed,
            "both traces must live in the graceful shutdown loop itself",
        );
    }

    /// Task item 3: existing behaviour unchanged. Every construction site in the
    /// terminal manager must still be `Owned` — increment 1 adds the ability to
    /// express adoption, it does not adopt anything.
    ///
    /// **Honest limitation: this is a source-text scan, and its blind spot has
    /// been measured, not guessed.** It matches the literal constructor names,
    /// so it catches the accident it exists for — increment 2's wiring arriving
    /// early by copy-paste — and nothing subtler. Mutating the construction
    /// site to `child.process_id().and_then(PtyChildHandle::adopt)` really does
    /// adopt, and this test really does stay GREEN, because the function is
    /// named without its parenthesis and the `owned` fallback keeps the first
    /// assertion true. A rename or a re-export would slip past the same way.
    /// It is a tripwire, not a proof of absence; the proof that production
    /// still owns its children is [`killing_an_owned_child_actually_ends_it`]
    /// and the behavioural locks beside it.
    #[test]
    fn the_terminal_manager_still_only_constructs_owned_children() {
        let terminal_src = include_str!("terminal.rs");
        assert!(
            terminal_src.contains("PtyChildHandle::owned(child)"),
            "the PTY construction site must build an OWNED handle",
        );
        for forbidden in [
            "PtyChildHandle::adopt(",
            "PtyChildHandle::adopt_with_start_time(",
        ] {
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

    // -----------------------------------------------------------------------
    // ReceivedMasterPty
    // -----------------------------------------------------------------------

    /// Open a pty pair and return `(master, slave)` as owned descriptors.
    fn open_pty_pair() -> (OwnedFd, OwnedFd) {
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
        // SAFETY: openpty just returned two owned descriptors.
        unsafe {
            (
                OwnedFd::from_raw_fd(master_fd),
                OwnedFd::from_raw_fd(slave_fd),
            )
        }
    }

    /// A real adopted PTY: open one, adopt it, and prove the received-master
    /// operations the spike proved still work through the product types.
    #[test]
    fn received_master_resizes_and_reports_its_fd() {
        let (master_owned, _slave) = open_pty_pair();
        let master_fd = master_owned.as_raw_fd();

        let me = std::process::id();
        let start_time = process_start_time(me).expect("readable");
        let master = ReceivedMasterPty::new(master_owned, me, start_time);

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

    /// Spike step 6, ported: the resize is only real if a process running on the
    /// SLAVE sees it. A same-fd `TIOCSWINSZ`/`TIOCGWINSZ` round trip would pass
    /// even if the kernel stored the value somewhere the child never reads.
    #[test]
    fn resizing_an_adopted_master_is_seen_by_a_process_on_the_slave() {
        let (master_owned, slave) = open_pty_pair();
        let me = std::process::id();
        let start_time = process_start_time(me).expect("readable");
        let master = ReceivedMasterPty::new(master_owned, me, start_time);

        // A fresh openpty has no window size at all, so 40x100 cannot be a
        // default leaking through.
        master
            .resize(PtySize {
                rows: 40,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("resize an adopted pty");

        let child_stdin = slave.try_clone().expect("dup the slave for the child");
        let output = std::process::Command::new("stty")
            .arg("size")
            .stdin(std::process::Stdio::from(child_stdin))
            .output()
            .expect("run stty on the slave side");
        assert!(
            output.status.success(),
            "stty failed: {}",
            String::from_utf8_lossy(&output.stderr),
        );
        let reported = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert_eq!(
            reported, "40 100",
            "a process reading the SLAVE must see the size we set on the received master",
        );
    }

    /// The duplicated master must be close-on-exec. Plain `dup(2)` clears the
    /// flag, which leaks the PTY master into every process the daemon spawns —
    /// and a leaked master holds the pty open, so the shell never sees EOF.
    #[test]
    fn a_duplicated_master_is_close_on_exec_and_is_not_inherited() {
        let (master_owned, _slave) = open_pty_pair();
        let me = std::process::id();
        let start_time = process_start_time(me).expect("readable");
        let master = ReceivedMasterPty::new(master_owned, me, start_time);

        let duped = master.dup_file().expect("duplicate the master");
        let duped_fd = duped.as_raw_fd();
        let flags = unsafe { libc::fcntl(duped_fd, libc::F_GETFD) };
        assert!(flags >= 0, "F_GETFD failed: {}", IoError::last_os_error());
        assert_ne!(
            flags & libc::FD_CLOEXEC,
            0,
            "the duplicated master must carry FD_CLOEXEC; dup(2) clears it and every child the \
             daemon spawns would then hold the pty open",
        );

        // …and prove it behaviourally: after exec, the child must not hold it.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn a child that would inherit a leaked fd");
        let child_pid = child.id();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut execed = false;
        while Instant::now() < deadline {
            if std::fs::read_to_string(format!("/proc/{child_pid}/comm"))
                .is_ok_and(|comm| comm.trim() == "sleep")
            {
                execed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let inherited = std::fs::read_link(format!("/proc/{child_pid}/fd/{duped_fd}")).ok();
        let _ = child.kill();
        let _ = child.wait();

        assert!(execed, "the probe child never reached exec");
        assert_eq!(
            inherited, None,
            "fd {duped_fd} survived exec into the child as {inherited:?} — the master leaked",
        );
    }

    /// A master read after the last slave closes returns `EIO` on Linux. Every
    /// caller here means end-of-stream by that, so the reader must say `Ok(0)`
    /// exactly as `portable_pty`'s own `PtyFd` does — otherwise a normal adopted
    /// session exit surfaces as an io error in the user's viewport.
    #[test]
    fn the_adopted_reader_reports_eio_as_end_of_stream() {
        let (master_owned, slave) = open_pty_pair();
        let me = std::process::id();
        let start_time = process_start_time(me).expect("readable");
        let master = ReceivedMasterPty::new(master_owned, me, start_time);

        let mut reader = master.try_clone_reader().expect("clone a reader");
        drop(slave); // the last slave descriptor: the pty now hangs up

        let mut buf = [0u8; 32];
        let read = reader.read(&mut buf);
        assert!(
            matches!(read, Ok(0)),
            "a hung-up adopted master must read as EOF, got {read:?}",
        );
    }

    /// `MasterPty::take_writer` documents that dropping the writer sends EOF to
    /// the slave. `portable_pty`'s `UnixMasterWriter` does that by writing a
    /// newline and the termios `VEOF` byte — EOF is only interpreted at the
    /// start of a line — and an adopted master must behave identically or the
    /// shell is left waiting for input that never comes.
    #[test]
    fn dropping_the_adopted_writer_sends_eot_to_the_slave() {
        let (master_owned, slave) = open_pty_pair();

        // Raw mode on the slave so the line discipline hands the bytes through
        // verbatim instead of interpreting them as line editing.
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        assert_eq!(
            unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut termios) },
            0,
            "tcgetattr on the slave failed: {}",
            IoError::last_os_error(),
        );
        let veof = termios.c_cc[libc::VEOF];
        assert_ne!(veof, 0, "the pty must have a VEOF character to send");
        unsafe { libc::cfmakeraw(&mut termios) };
        assert_eq!(
            unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &termios) },
            0,
            "tcsetattr on the slave failed: {}",
            IoError::last_os_error(),
        );

        let me = std::process::id();
        let start_time = process_start_time(me).expect("readable");
        let master = ReceivedMasterPty::new(master_owned, me, start_time);

        // A BOUNDED read: a writer that sends nothing must FAIL this test, not
        // hang it. A blocking slave read has nothing to wait for once the write
        // is gone — the master fd is still open, so no hangup ever arrives — and
        // a stuck suite is not a lock.
        let flags = unsafe { libc::fcntl(slave.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0, "F_GETFL failed: {}", IoError::last_os_error());
        assert_eq!(
            unsafe { libc::fcntl(slave.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0,
            "F_SETFL(O_NONBLOCK) failed: {}",
            IoError::last_os_error(),
        );

        let writer = master.take_writer().expect("take the writer");
        drop(writer);
        let mut slave_file = File::from(slave);
        let mut buf = [0u8; 16];
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        while seen.len() < 2 && Instant::now() < deadline {
            match slave_file.read(&mut buf) {
                Ok(0) => break,
                Ok(read) => seen.extend_from_slice(&buf[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("reading the slave side failed: {error}"),
            }
        }
        assert_eq!(
            seen.as_slice(),
            &[b'\n', veof],
            "dropping the writer must send newline + VEOF, as portable_pty's UnixMasterWriter \
             does; the trait's contract is that dropping the writer sends EOF",
        );
    }
}
