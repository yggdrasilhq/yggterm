//! O7 — SCM_RIGHTS PTY fd-handoff spike.
//!
//! The constitution's durable half: a plain shell pins its daemon forever
//! because a PTY cannot move between daemons. `docs/pending-bugs.md` (search
//! "LEVEL (b) — LOSSLESS") maps where this would slot in and names the actual
//! design question — **not** the `sendmsg` call, but WHO OWNS THE CHILD after
//! the fd moves.
//!
//! This proves the three risky primitives outside the product:
//!
//! 1. **Send/receive** — `sendmsg`/`recvmsg` with `SCM_RIGHTS` moving a PTY
//!    MASTER fd between two processes over a `UnixStream`, out-of-band from any
//!    JSON protocol (the daemon wire is one JSON line per request and has no
//!    room for ancillary data).
//! 2. **The receive side** — the map calls this the expensive half. Drive a
//!    real `bash -i` from the RECEIVED RAW FD: write to it, read its output,
//!    and resize it (`TIOCSWINSZ`).
//! 3. **The ownership decision** — the SENDER EXITS after the transfer. The
//!    shell re-parents to init, the receiver cannot `waitpid` it and tracks
//!    liveness through `/proc` instead, and the PTY keeps working: a command
//!    typed AFTER the sender is gone still runs and still answers.
//!
//! Run: `pty-fd-handoff-spike` (orchestrates both halves). Exit 0 = PASS.

use std::ffi::{CString, c_char, c_int, c_void};

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, ExitCode};
use std::ptr;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Hand-written externs
// ---------------------------------------------------------------------------

const TIOCSWINSZ: u64 = 0x5414;
const TIOCGWINSZ: u64 = 0x5413;
const SOL_SOCKET: c_int = 1;
const SCM_RIGHTS: c_int = 1;
const O_NONBLOCK: c_int = 0o4000;
const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;
const EAGAIN: c_int = 11;

#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
struct WinSize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

#[repr(C)]
struct IoVec {
    iov_base: *mut c_void,
    iov_len: usize,
}

/// `repr(C)` reproduces glibc's padding for us: `msg_namelen` is a `u32`
/// followed by 4 bytes of padding before the next pointer, and `msg_flags`
/// trails the struct. Getting this wrong is the classic SCM_RIGHTS bug.
#[repr(C)]
struct MsgHdr {
    msg_name: *mut c_void,
    msg_namelen: u32,
    msg_iov: *mut IoVec,
    msg_iovlen: usize,
    msg_control: *mut c_void,
    msg_controllen: usize,
    msg_flags: c_int,
}

#[repr(C)]
struct CmsgHdr {
    cmsg_len: usize,
    cmsg_level: c_int,
    cmsg_type: c_int,
}

/// `CMSG_SPACE(sizeof(int))` on 64-bit Linux: a 16-byte header plus a 4-byte
/// payload rounded up to the 8-byte cmsg alignment.
const CMSG_SPACE_ONE_FD: usize = 24;
/// `CMSG_LEN(sizeof(int))` — header plus the payload, NOT rounded.
const CMSG_LEN_ONE_FD: usize = 20;

unsafe extern "C" {
    fn forkpty(
        amaster: *mut c_int,
        name: *mut c_char,
        termp: *const c_void,
        winp: *const WinSize,
    ) -> c_int;
    fn execvp(file: *const c_char, argv: *const *const c_char) -> c_int;
    fn _exit(status: c_int) -> !;
    fn sendmsg(sockfd: c_int, msg: *const MsgHdr, flags: c_int) -> isize;
    fn recvmsg(sockfd: c_int, msg: *mut MsgHdr, flags: c_int) -> isize;
    fn ioctl(fd: c_int, request: u64, ...) -> c_int;
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
    fn kill(pid: c_int, sig: c_int) -> c_int;
    fn __errno_location() -> *mut c_int;
}

fn errno() -> c_int {
    unsafe { *__errno_location() }
}

// ---------------------------------------------------------------------------
// Primitive 1 — SCM_RIGHTS
// ---------------------------------------------------------------------------

/// Send `fd` plus a small text payload over `stream`.
///
/// The payload rides the SAME `sendmsg` as the ancillary data on purpose: the
/// receiver needs the shell's pid to track liveness, and a second channel could
/// arrive out of order with the fd.
fn send_fd(stream: &UnixStream, fd: c_int, payload: &str) -> Result<(), String> {
    let bytes = payload.as_bytes();
    let mut iov = IoVec {
        iov_base: bytes.as_ptr() as *mut c_void,
        iov_len: bytes.len(),
    };
    let mut control = [0u8; CMSG_SPACE_ONE_FD];

    unsafe {
        let cmsg = control.as_mut_ptr() as *mut CmsgHdr;
        (*cmsg).cmsg_len = CMSG_LEN_ONE_FD;
        (*cmsg).cmsg_level = SOL_SOCKET;
        (*cmsg).cmsg_type = SCM_RIGHTS;
        // CMSG_DATA: immediately after the header.
        let data = (cmsg as *mut u8).add(std::mem::size_of::<CmsgHdr>()) as *mut c_int;
        ptr::write_unaligned(data, fd);

        let msg = MsgHdr {
            msg_name: ptr::null_mut(),
            msg_namelen: 0,
            msg_iov: &mut iov,
            msg_iovlen: 1,
            msg_control: control.as_mut_ptr() as *mut c_void,
            msg_controllen: CMSG_SPACE_ONE_FD,
            msg_flags: 0,
        };
        let sent = sendmsg(stream.as_raw_fd(), &msg, 0);
        if sent < 0 {
            return Err(format!("sendmsg failed: errno {}", errno()));
        }
    }
    Ok(())
}

/// Receive one fd plus its payload. Returns an owned fd so the spike cannot
/// leak it — the real integration wants the same discipline.
fn recv_fd(stream: &UnixStream) -> Result<(OwnedFd, String), String> {
    let mut buf = [0u8; 256];
    let mut iov = IoVec {
        iov_base: buf.as_mut_ptr() as *mut c_void,
        iov_len: buf.len(),
    };
    let mut control = [0u8; CMSG_SPACE_ONE_FD];

    unsafe {
        let mut msg = MsgHdr {
            msg_name: ptr::null_mut(),
            msg_namelen: 0,
            msg_iov: &mut iov,
            msg_iovlen: 1,
            msg_control: control.as_mut_ptr() as *mut c_void,
            msg_controllen: CMSG_SPACE_ONE_FD,
            msg_flags: 0,
        };
        let got = recvmsg(stream.as_raw_fd(), &mut msg, 0);
        if got < 0 {
            return Err(format!("recvmsg failed: errno {}", errno()));
        }
        if msg.msg_controllen < CMSG_LEN_ONE_FD {
            return Err(format!(
                "no ancillary data arrived (msg_controllen={}) — the fd did NOT travel",
                msg.msg_controllen
            ));
        }
        let cmsg = control.as_ptr() as *const CmsgHdr;
        if (*cmsg).cmsg_level != SOL_SOCKET || (*cmsg).cmsg_type != SCM_RIGHTS {
            return Err("ancillary data is not SCM_RIGHTS".to_string());
        }
        let data = (cmsg as *const u8).add(std::mem::size_of::<CmsgHdr>()) as *const c_int;
        let fd = ptr::read_unaligned(data);
        if fd < 0 {
            return Err(format!("received a negative fd: {fd}"));
        }
        let payload = String::from_utf8_lossy(&buf[..got as usize]).into_owned();
        Ok((OwnedFd::from_raw_fd(fd), payload))
    }
}

// ---------------------------------------------------------------------------
// Primitive 2 — drive the PTY from the received raw fd
// ---------------------------------------------------------------------------

fn set_nonblocking(fd: c_int) -> Result<(), String> {
    unsafe {
        let flags = fcntl(fd, F_GETFL, 0);
        if flags < 0 || fcntl(fd, F_SETFL, flags | O_NONBLOCK) < 0 {
            return Err(format!("fcntl O_NONBLOCK failed: errno {}", errno()));
        }
    }
    Ok(())
}

fn pty_write(fd: c_int, text: &str) -> Result<(), String> {
    let bytes = text.as_bytes();
    let mut written = 0usize;
    while written < bytes.len() {
        let n = unsafe {
            write(
                fd,
                bytes[written..].as_ptr() as *const c_void,
                bytes.len() - written,
            )
        };
        if n < 0 {
            if errno() == EAGAIN {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            return Err(format!("write to pty failed: errno {}", errno()));
        }
        written += n as usize;
    }
    Ok(())
}

/// Read until `needle` appears or the deadline passes. Returns everything read.
fn pty_read_until(fd: c_int, needle: &str, timeout: Duration) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    let mut out = String::new();
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        let n = unsafe { read(fd, buf.as_mut_ptr() as *mut c_void, buf.len()) };
        if n > 0 {
            out.push_str(&String::from_utf8_lossy(&buf[..n as usize]));
            if out.contains(needle) {
                return Ok(out);
            }
        } else if n < 0 && errno() != EAGAIN {
            return Err(format!("read from pty failed: errno {}", errno()));
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    Err(format!(
        "timed out waiting for {needle:?}; got {} bytes: {:?}",
        out.len(),
        out.chars().rev().take(200).collect::<String>()
    ))
}

fn pty_resize(fd: c_int, rows: u16, cols: u16) -> Result<(), String> {
    let size = WinSize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    if unsafe { ioctl(fd, TIOCSWINSZ, &size) } < 0 {
        return Err(format!("TIOCSWINSZ failed: errno {}", errno()));
    }
    Ok(())
}

fn pty_window(fd: c_int) -> Result<WinSize, String> {
    let mut size = WinSize::default();
    if unsafe { ioctl(fd, TIOCGWINSZ, &mut size) } < 0 {
        return Err(format!("TIOCGWINSZ failed: errno {}", errno()));
    }
    Ok(size)
}

// ---------------------------------------------------------------------------
// Primitive 3 — liveness without waitpid
// ---------------------------------------------------------------------------

fn proc_status_field(pid: i32, field: &str) -> Option<String> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    text.lines()
        .find(|line| line.starts_with(field))
        .map(|line| line.split_whitespace().last().unwrap_or("").to_string())
}

/// The receiver's replacement for `waitpid`: the shell is NOT its child, so the
/// only liveness signals available are `/proc` and `kill(pid, 0)`.
fn process_is_alive(pid: i32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists() && unsafe { kill(pid, 0) } == 0
}

// ---------------------------------------------------------------------------
// The sender half
// ---------------------------------------------------------------------------

fn run_sender(socket_path: &str) -> ExitCode {
    let mut master: c_int = -1;
    let pid = unsafe { forkpty(&mut master, ptr::null_mut(), ptr::null(), ptr::null()) };
    if pid < 0 {
        eprintln!("[send] forkpty failed: errno {}", errno());
        return ExitCode::from(1);
    }
    if pid == 0 {
        // Child: the slave is already our controlling terminal. Only
        // async-signal-safe work before exec.
        let file = CString::new("bash").unwrap();
        let arg0 = CString::new("bash").unwrap();
        let arg1 = CString::new("-i").unwrap();
        let argv = [arg0.as_ptr(), arg1.as_ptr(), ptr::null()];
        unsafe {
            execvp(file.as_ptr(), argv.as_ptr());
            _exit(127);
        }
    }

    eprintln!("[send] pid={} spawned bash -i as pid {pid} on master fd {master}", std::process::id());
    let stream = match UnixStream::connect(socket_path) {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("[send] connect failed: {err}");
            return ExitCode::from(1);
        }
    };
    // The payload carries what the fd cannot: which pid the receiver must watch.
    if let Err(err) = send_fd(&stream, master, &format!("shell_pid={pid}")) {
        eprintln!("[send] {err}");
        return ExitCode::from(1);
    }
    eprintln!("[send] master fd sent; exiting so the shell re-parents");
    // Deliberately exit WITHOUT waiting for the shell: that is the ownership
    // decision under test.
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// The receiver half (also the orchestrator)
// ---------------------------------------------------------------------------

fn fail(step: &str, err: String) -> ExitCode {
    eprintln!("[recv] FAIL at {step}: {err}");
    println!("[spike] ACCEPTANCE=FAIL");
    ExitCode::from(1)
}

fn run_receiver() -> ExitCode {
    let socket_path = format!("/tmp/pty-fd-handoff-{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);
    let listener = match UnixListener::bind(&socket_path) {
        Ok(listener) => listener,
        Err(err) => return fail("bind", err.to_string()),
    };

    let exe = std::env::current_exe().expect("current exe");
    let mut sender = match Command::new(&exe).args(["send", &socket_path]).spawn() {
        Ok(child) => child,
        Err(err) => return fail("spawn sender", err.to_string()),
    };
    let sender_pid = sender.id() as i32;

    let (stream, _) = match listener.accept() {
        Ok(pair) => pair,
        Err(err) => return fail("accept", err.to_string()),
    };
    let (owned_fd, payload) = match recv_fd(&stream) {
        Ok(pair) => pair,
        Err(err) => return fail("recvmsg", err),
    };
    let master = owned_fd.as_raw_fd();
    let shell_pid: i32 = payload
        .trim()
        .strip_prefix("shell_pid=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(-1);
    println!("[spike] 1. SCM_RIGHTS: received master fd {master}, payload {payload:?}");
    if shell_pid <= 0 {
        return fail("payload", format!("no shell pid in {payload:?}"));
    }

    if let Err(err) = set_nonblocking(master) {
        return fail("nonblocking", err);
    }

    // ---- the sender EXITS. This is the ownership decision under test. ----
    match sender.wait() {
        Ok(status) => println!("[spike] 2. sender exited: {status}"),
        Err(err) => return fail("sender wait", err.to_string()),
    }
    // Give the kernel a moment to re-parent the orphan.
    std::thread::sleep(Duration::from_millis(200));
    if process_is_alive(sender_pid) {
        return fail("sender liveness", "the sender is still alive".to_string());
    }

    let ppid = proc_status_field(shell_pid, "PPid:").unwrap_or_default();
    let alive = process_is_alive(shell_pid);
    println!(
        "[spike] 3. shell pid {shell_pid}: alive={alive} PPid={ppid} (was {sender_pid}) \
         — tracked via /proc, NOT waitpid"
    );
    if !alive {
        return fail("shell liveness", "the shell died with its sender".to_string());
    }
    if ppid == sender_pid.to_string() {
        return fail("re-parent", "the shell did not re-parent".to_string());
    }

    // ---- primitive 2: write / read / resize from the RECEIVED fd ----
    if let Err(err) = pty_write(master, "printf 'MARK%s\\n' AFTER-HANDOFF\n") {
        return fail("write", err);
    }
    match pty_read_until(master, "MARKAFTER-HANDOFF", Duration::from_secs(10)) {
        Ok(_) => println!(
            "[spike] 4. WROTE to the received fd and READ the shell's answer \
             (MARKAFTER-HANDOFF) — with the sender already gone"
        ),
        Err(err) => return fail("read after handoff", err),
    }

    let before = match pty_window(master) {
        Ok(size) => size,
        Err(err) => return fail("TIOCGWINSZ", err),
    };
    if let Err(err) = pty_resize(master, 40, 100) {
        return fail("TIOCSWINSZ", err);
    }
    let after = match pty_window(master) {
        Ok(size) => size,
        Err(err) => return fail("TIOCGWINSZ", err),
    };
    println!(
        "[spike] 5. RESIZED via the received fd: {}x{} -> {}x{}",
        before.ws_col, before.ws_row, after.ws_col, after.ws_row
    );
    if after.ws_row != 40 || after.ws_col != 100 {
        return fail("resize", format!("kernel reports {after:?}"));
    }
    // …and the SHELL must agree, or we only moved a kernel struct.
    if let Err(err) = pty_write(master, "stty size\n") {
        return fail("write stty", err);
    }
    match pty_read_until(master, "40 100", Duration::from_secs(10)) {
        Ok(_) => println!("[spike] 6. the SHELL sees the new size (stty size -> 40 100)"),
        Err(err) => return fail("stty size", err),
    }

    // ---- one more command, long after the sender died ----
    if let Err(err) = pty_write(master, "printf 'STILL%s\\n' -ALIVE\n") {
        return fail("write final", err);
    }
    match pty_read_until(master, "STILL-ALIVE", Duration::from_secs(10)) {
        Ok(_) => println!("[spike] 7. the PTY still works with no owning process anywhere"),
        Err(err) => return fail("final read", err),
    }

    // Clean up: the shell is nobody's child, so it must be killed explicitly.
    unsafe { kill(shell_pid, 9) };
    let _ = std::fs::remove_file(&socket_path);

    println!("[spike] ACCEPTANCE=PASS");
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("send") => match args.get(1) {
            Some(socket) => run_sender(socket),
            None => {
                eprintln!("usage: pty-fd-handoff-spike send <socket>");
                ExitCode::from(2)
            }
        },
        _ => run_receiver(),
    }
}
