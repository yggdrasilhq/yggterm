//! Level (b), increment 2, step 1: moving a PTY master fd between daemons.
//!
//! **Why this exists.** [`crate::pty_adoption`] supplies the two types that
//! make an adopted PTY *expressible* — `PtyChildHandle::Adopted` and
//! `ReceivedMasterPty` — and says in its own header that nothing is wired to
//! the daemon's handoff yet. This module is the wire those types were waiting
//! for: the one call that moves a master fd from the daemon that spawned it to
//! the daemon that will drive it.
//!
//! Until it existed, `hot update handoff` was a message rather than a
//! mechanism. A daemon owning live terminals answered *"handoff started:
//! preserving N live terminal runtime(s)"*, registered a successor, and then
//! kept every PTY forever, because there was nothing that could carry one
//! across. Measured on the live desktop host 2026-08-04: five daemons all
//! reported a started handoff, `spawn_ok: true`, `successor_already_live:
//! true`, and afterwards the predecessor still held all 14 of its PTYs while
//! the successor held none — and the session rail still read *"daemon is on
//! 3.0.22 · older than this client"*.
//!
//! ## The two decisions this module inherits, and does not re-open
//!
//! Both are settled in `docs/settled-calls.md` under LEVEL (b):
//!
//! 1. **The transcript payload travels BEFORE the fd.** So this module moves
//!    exactly one fd plus a SMALL token, never the scrollback: the caller
//!    writes the transcript as an ordinary line first, and the token here is
//!    only what the receiver cannot learn from the fd itself (the child's pid
//!    and start time — see rule 3 below).
//! 2. **`sendmsg` success is the commit point.** After it returns, the fd
//!    belongs to the successor and nothing downstream may be recovered by
//!    re-sending. [`send_master_fd`] is therefore deliberately not retryable
//!    and its caller must treat an `Ok` as irreversible.
//!
//! ## Why this uses `libc`'s `msghdr` and not the spike's
//!
//! The spike (`docs/spikes/pty-fd-handoff/`) hand-rolled `#[repr(C)]`
//! `msghdr`/`cmsghdr` because it was a standalone crate with no dependencies,
//! and its README names that layout as *"the classic place this goes wrong"* —
//! `msg_namelen` is a `u32` followed by four bytes of padding before the next
//! pointer. This crate already depends on `libc`, so the struct, the padding
//! and `CMSG_SPACE`/`CMSG_LEN`/`CMSG_FIRSTHDR`/`CMSG_DATA` all come from the
//! platform definition and the hazard is gone rather than reproduced.
//!
//! One thing the spike did NOT do and this does: **`MSG_CMSG_CLOEXEC`**. A
//! received master fd must not survive an exec, for the same reason
//! `ReceivedMasterPty` uses `F_DUPFD_CLOEXEC` and never a plain `dup` — a
//! leaked master means the slave's hangup never arrives and the shell never
//! sees EOF.
//!
//! ## Linux-gated at the function, like adoption is at the variant
//!
//! `SCM_RIGHTS` over `AF_UNIX` is the mechanism; the non-Linux arms return a
//! named error instead of failing to compile, so one module owns the concept
//! on every target and no second encoding appears.

use std::io;
use std::os::fd::{OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

/// The largest token [`send_master_fd`] will carry beside the fd, and the
/// buffer [`recv_master_fd`] reads into.
///
/// Small on purpose: the transcript is a separate, earlier write (decision 1
/// above). This carries identity, not content.
pub const HANDOFF_TOKEN_MAX_BYTES: usize = 512;

/// The receiver saw a message with no ancillary data at all.
///
/// This is the spike's negative control promoted to a named error: a check
/// that can only pass is worth nothing, so the one failure that means *the fd
/// did not travel* is distinguishable from every other I/O error.
pub const ERR_NO_ANCILLARY: &str =
    "no ancillary data arrived — the fd did NOT travel";

#[cfg(target_os = "linux")]
mod imp {
    use super::{ERR_NO_ANCILLARY, HANDOFF_TOKEN_MAX_BYTES};
    use std::io;
    use std::os::fd::{FromRawFd, OwnedFd, RawFd};
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::UnixStream;

    pub fn send_master_fd(stream: &UnixStream, fd: RawFd, token: &[u8]) -> io::Result<()> {
        if token.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "handoff token must be non-empty: a zero-length payload is \
                 indistinguishable from a peer that closed",
            ));
        }
        if token.len() > HANDOFF_TOKEN_MAX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "handoff token is {} bytes, over the {HANDOFF_TOKEN_MAX_BYTES}-byte \
                     limit; the transcript travels as its own line BEFORE the fd",
                    token.len()
                ),
            ));
        }

        let mut iov = libc::iovec {
            iov_base: token.as_ptr() as *mut libc::c_void,
            iov_len: token.len(),
        };
        let control_len = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
        let mut control = vec![0u8; control_len];

        // SAFETY: `msg` is zeroed then fully initialised below; `control` and
        // `iov` outlive the call; the cmsg header is written through
        // `CMSG_FIRSTHDR` on a buffer sized by `CMSG_SPACE`.
        let sent = unsafe {
            let mut msg: libc::msghdr = std::mem::zeroed();
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            msg.msg_control = control.as_mut_ptr() as *mut libc::c_void;
            msg.msg_controllen = control_len as _;

            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            if cmsg.is_null() {
                return Err(io::Error::other(
                    "CMSG_FIRSTHDR returned null on a CMSG_SPACE-sized buffer",
                ));
            }
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
            std::ptr::write_unaligned(libc::CMSG_DATA(cmsg) as *mut RawFd, fd);

            libc::sendmsg(stream.as_raw_fd(), &msg, 0)
        };

        if sent < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn recv_master_fd(stream: &UnixStream) -> io::Result<(OwnedFd, Vec<u8>)> {
        let mut buf = [0u8; HANDOFF_TOKEN_MAX_BYTES];
        let mut iov = libc::iovec {
            iov_base: buf.as_mut_ptr() as *mut libc::c_void,
            iov_len: buf.len(),
        };
        let control_len = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
        let mut control = vec![0u8; control_len];

        // SAFETY: as above; the received fd is wrapped in an `OwnedFd` before
        // any early return past the point it was extracted, so it cannot leak.
        unsafe {
            let mut msg: libc::msghdr = std::mem::zeroed();
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            msg.msg_control = control.as_mut_ptr() as *mut libc::c_void;
            msg.msg_controllen = control_len as _;

            // MSG_CMSG_CLOEXEC: a received master must not survive an exec, or
            // the slave's hangup never arrives and the shell never sees EOF.
            let got = libc::recvmsg(stream.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC);
            if got < 0 {
                return Err(io::Error::last_os_error());
            }

            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            if cmsg.is_null() || (msg.msg_controllen as usize) < control_len {
                return Err(io::Error::new(io::ErrorKind::InvalidData, ERR_NO_ANCILLARY));
            }
            if (*cmsg).cmsg_level != libc::SOL_SOCKET || (*cmsg).cmsg_type != libc::SCM_RIGHTS {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "ancillary data is not SCM_RIGHTS",
                ));
            }
            let raw = std::ptr::read_unaligned(libc::CMSG_DATA(cmsg) as *const RawFd);
            if raw < 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("received a negative fd: {raw}"),
                ));
            }
            // Own it immediately: every path below this line can return.
            let owned = OwnedFd::from_raw_fd(raw);
            if got == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the fd arrived with an empty token; identity cannot be established",
                ));
            }
            Ok((owned, buf[..got as usize].to_vec()))
        }
    }

    /// Send `token` with NO ancillary data — the negative control.
    ///
    /// Exists so the round-trip test can prove [`recv_master_fd`] REJECTS a
    /// message whose fd did not travel. Without it the success path could be
    /// passing for the wrong reason.
    #[cfg(test)]
    pub fn send_token_without_fd(stream: &UnixStream, token: &[u8]) -> io::Result<()> {
        let mut iov = libc::iovec {
            iov_base: token.as_ptr() as *mut libc::c_void,
            iov_len: token.len(),
        };
        // SAFETY: `msg` is zeroed then initialised with no control buffer.
        let sent = unsafe {
            let mut msg: libc::msghdr = std::mem::zeroed();
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            libc::sendmsg(stream.as_raw_fd(), &msg, 0)
        };
        if sent < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use std::io;
    use std::os::fd::{OwnedFd, RawFd};
    use std::os::unix::net::UnixStream;

    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "PTY fd handoff is Linux-only: it needs SCM_RIGHTS over AF_UNIX \
             plus /proc identity for the adopted child",
        )
    }

    pub fn send_master_fd(_s: &UnixStream, _fd: RawFd, _token: &[u8]) -> io::Result<()> {
        Err(unsupported())
    }

    pub fn recv_master_fd(_s: &UnixStream) -> io::Result<(OwnedFd, Vec<u8>)> {
        Err(unsupported())
    }
}

/// Move one PTY master fd to the peer, carrying `token` in the same `sendmsg`.
///
/// **`Ok(())` is the commit point.** The fd now belongs to the receiver; the
/// caller must drop its own runtime for that session without killing the child
/// and must never re-send. The token rides the same call as the ancillary data
/// on purpose: the receiver cannot learn the child's pid from the fd, and a
/// second channel could arrive out of order with it.
pub fn send_master_fd(stream: &UnixStream, fd: RawFd, token: &[u8]) -> io::Result<()> {
    imp::send_master_fd(stream, fd, token)
}

/// Receive one PTY master fd and its token.
///
/// Fails with [`ERR_NO_ANCILLARY`] when the peer sent a plain message — that
/// is the one failure meaning *the fd did not travel*, and it must never be
/// confused with a transport error.
pub fn recv_master_fd(stream: &UnixStream) -> io::Result<(OwnedFd, Vec<u8>)> {
    imp::recv_master_fd(stream)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;

    /// The whole point, end to end: an fd created by one side is USED by the
    /// other. A socketpair half stands in for the PTY master here — this test
    /// is about the wire, and `pty_adoption` already owns the master-specific
    /// proofs.
    #[test]
    fn a_received_fd_is_the_senders_fd_and_it_works() {
        let (a, b) = UnixStream::pair().expect("socketpair");
        let (carried, mut far_side) = UnixStream::pair().expect("carried pair");

        send_master_fd(&a, carried.as_raw_fd(), b"key=local://demo pid=42")
            .expect("sendmsg must succeed");
        drop(carried);

        let (received, token) = recv_master_fd(&b).expect("recvmsg must succeed");
        assert_eq!(token, b"key=local://demo pid=42");

        // Prove it is the SAME endpoint, not merely a valid descriptor: write
        // on the far side and read it out of the fd that crossed.
        far_side.write_all(b"AFTER-HANDOFF").expect("write");
        drop(far_side);
        let mut stream = UnixStream::from(received);
        let mut got = String::new();
        stream.read_to_string(&mut got).expect("read");
        assert_eq!(
            got, "AFTER-HANDOFF",
            "the received fd must be the sender's fd, still connected to its peer"
        );
    }

    /// The negative control, promoted from the spike. Without this the success
    /// case above could pass for the wrong reason.
    #[test]
    fn a_message_with_no_ancillary_data_is_refused_and_says_the_fd_did_not_travel() {
        let (a, b) = UnixStream::pair().expect("socketpair");
        imp::send_token_without_fd(&a, b"key=local://demo pid=42").expect("plain send");

        let err = recv_master_fd(&b).expect_err("a message with no fd must NOT succeed");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(
            err.to_string().contains(ERR_NO_ANCILLARY),
            "the refusal must name the cause, got: {err}"
        );
    }

    /// Decision 1: the transcript is its own line. A caller that tries to push
    /// the scrollback through the token is refused rather than silently
    /// truncated.
    #[test]
    fn a_token_over_the_limit_is_refused_because_the_transcript_is_a_separate_line() {
        let (a, _b) = UnixStream::pair().expect("socketpair");
        let (carried, _far) = UnixStream::pair().expect("carried pair");
        let huge = vec![b'x'; HANDOFF_TOKEN_MAX_BYTES + 1];

        let err = send_master_fd(&a, carried.as_raw_fd(), &huge)
            .expect_err("an oversized token must be refused");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("BEFORE the fd"));
    }

    /// An empty token cannot be told apart from a closed peer, so it is a
    /// programming error rather than a runtime surprise.
    #[test]
    fn an_empty_token_is_refused_at_the_sender() {
        let (a, _b) = UnixStream::pair().expect("socketpair");
        let (carried, _far) = UnixStream::pair().expect("carried pair");

        let err = send_master_fd(&a, carried.as_raw_fd(), b"")
            .expect_err("an empty token must be refused");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }
}
