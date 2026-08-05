//! Level (b), increment 2, step 3: the handoff PROTOCOL.
//!
//! [`crate::pty_handoff_wire`] moves one fd. [`crate::terminal`] can install a
//! runtime around a received one. This is what the two say to each other.
//!
//! **Why a separate socket.** The daemon's request socket carries JSON lines
//! and its handler has no way to `recvmsg` ancillary data mid-request. Rather
//! than teach every request path about file descriptors, a handoff gets its own
//! listener, named for the daemon's version exactly like the request socket —
//! so a predecessor that already knows the successor's VERSION (it computes
//! `live_successor_version` before it lingers) can derive the path with no
//! discovery step.
//!
//! ## The order on the wire, and why it is not negotiable
//!
//! Settled in `docs/settled-calls.md`: **the transcript travels BEFORE the fd,
//! and `sendmsg` success is the commit point.**
//!
//! ```text
//! predecessor                              successor
//!     |  metadata line (JSON + screen)  ->  |   parse; still recoverable here
//!     |  sendmsg(master fd, token)      ->  |   COMMIT POINT
//!     |  <- ack line (adopted / error)      |
//! ```
//!
//! Everything before the `sendmsg` is recoverable: if the metadata is rejected
//! the predecessor still owns its PTY and simply keeps it. **After** the
//! `sendmsg` the descriptor belongs to the successor and there is no way back —
//! which is why the ack exists at all. The ack does not undo anything; it tells
//! the predecessor whether it may now drop its runtime, or whether it has just
//! handed a live shell to a daemon that could not seat it and must say so
//! loudly rather than silently orphan the user's session.
//!
//! ## What travels, and what cannot
//!
//! The token beside the fd carries the child's `(pid, start_time)` because the
//! receiver cannot learn either from the descriptor, and because a bare pid is
//! not an identity — see [`crate::pty_adoption`]. The screen travels in the
//! metadata line because the fd alone hands over a live terminal with an empty
//! transcript.
//!
//! The `Child` handle does not travel and cannot: it is the predecessor's
//! direct child. After the move the successor drives the PTY but can never
//! `waitpid` it, which is the whole reason `PtyChildHandle` is an enum.

use std::io::{BufRead, BufReader, Write};

use std::os::fd::{OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::pty_handoff_wire::{HANDOFF_TOKEN_MAX_BYTES, recv_master_fd, send_master_fd};

/// Everything the successor needs that it cannot read off the descriptor.
///
/// `screen` is the predecessor's formatted vt100 screen. It is the largest
/// field by far and it is why this is a line of its own rather than part of the
/// 512-byte token.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct HandoffMetadata {
    /// Wire version. A successor that does not recognise it refuses BEFORE the
    /// commit point, which is the only place a refusal is free.
    pub version: u32,
    pub runtime_key: String,
    pub launch_command: String,
    pub cwd: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub shell_pid: u32,
    pub shell_start_time: u64,
    pub screen: String,
}

/// The only wire version this build speaks.
pub(crate) const HANDOFF_WIRE_VERSION: u32 = 1;

/// The handoff listener's address, named for the daemon's version exactly like
/// the request socket, so a predecessor can derive it from a version string.
pub(crate) fn handoff_socket_path(home_dir: &Path, version: &str) -> PathBuf {
    home_dir.join(format!("pty-handoff-{}.sock", version.replace('.', "-")))
}

/// The token that rides the same `sendmsg` as the descriptor.
///
/// Deliberately tiny and self-describing: it repeats the identity from the
/// metadata so the receiver can refuse a descriptor that does not match the
/// line it just read, rather than seating a PTY against the wrong record.
pub(crate) fn handoff_token(runtime_key: &str, shell_pid: u32, shell_start_time: u64) -> String {
    format!("key={runtime_key} pid={shell_pid} start={shell_start_time}")
}

/// Hand one PTY to the successor listening on `socket_path`.
///
/// ⛔ **`Ok(())` means the descriptor is GONE.** The caller must drop its
/// runtime without killing the child, and must never re-send. An `Err` before
/// the commit point means nothing moved and the caller still owns its PTY; an
/// `Err` after it names a session that is now the successor's problem, and the
/// two cases are distinguished by [`HandoffError::committed`].
pub(crate) fn send_session(
    socket_path: &Path,
    metadata: &HandoffMetadata,
    master_fd: RawFd,
) -> std::result::Result<(), HandoffError> {
    let mut stream = UnixStream::connect(socket_path).map_err(|error| HandoffError {
        committed: false,
        message: format!(
            "connecting to successor handoff socket {}: {error}",
            socket_path.display()
        ),
    })?;

    let mut line = serde_json::to_string(metadata).map_err(|error| HandoffError {
        committed: false,
        message: format!("encoding handoff metadata: {error}"),
    })?;
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|error| HandoffError {
            committed: false,
            message: format!("sending handoff metadata: {error}"),
        })?;

    let token = handoff_token(
        &metadata.runtime_key,
        metadata.shell_pid,
        metadata.shell_start_time,
    );
    // THE COMMIT POINT.
    send_master_fd(&stream, master_fd, token.as_bytes()).map_err(|error| HandoffError {
        committed: false,
        message: format!("sending master fd: {error}"),
    })?;

    // Past here every failure is a REPORT, not a recovery: the descriptor is
    // the successor's now whatever the ack says.
    let mut reader = BufReader::new(&stream);
    let mut ack_line = String::new();
    if reader.read_line(&mut ack_line).is_err() || ack_line.trim().is_empty() {
        return Err(HandoffError {
            committed: true,
            message: "successor accepted the fd but never acknowledged it".to_string(),
        });
    }
    let ack: HandoffAck = serde_json::from_str(ack_line.trim()).map_err(|error| HandoffError {
        committed: true,
        message: format!("successor sent an unreadable ack ({error}): {ack_line:?}"),
    })?;
    if !ack.adopted {
        return Err(HandoffError {
            committed: true,
            message: format!(
                "successor took the fd and refused to seat it: {}",
                ack.error.unwrap_or_else(|| "no reason given".to_string())
            ),
        });
    }
    Ok(())
}

/// Whether a failed handoff left the descriptor behind or took it.
#[derive(Debug, Clone)]
pub(crate) struct HandoffError {
    /// `false` — nothing moved, the caller still owns its PTY and may retry.
    /// `true` — the fd is gone and the session is the successor's; the caller
    /// must NOT keep driving it and must NOT re-send.
    pub committed: bool,
    pub message: String,
}

impl std::fmt::Display for HandoffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({})",
            self.message,
            if self.committed {
                "AFTER the commit point — the fd is gone"
            } else {
                "before the commit point — the fd stayed"
            }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HandoffAck {
    pub adopted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Read one handoff from an accepted connection: the metadata line, then the
/// descriptor.
///
/// Refuses an unknown wire version and a token that disagrees with the line —
/// both BEFORE returning, so a mismatched descriptor is closed by `OwnedFd`'s
/// drop rather than seated against the wrong record.
pub(crate) fn receive_session(stream: &UnixStream) -> Result<(HandoffMetadata, OwnedFd)> {
    let line = read_line_without_overreading(stream).context("reading handoff metadata line")?;
    if line.trim().is_empty() {
        bail!("handoff connection closed before sending metadata");
    }
    let metadata: HandoffMetadata =
        serde_json::from_str(line.trim()).context("decoding handoff metadata")?;
    if metadata.version != HANDOFF_WIRE_VERSION {
        bail!(
            "handoff wire version {} is not {HANDOFF_WIRE_VERSION}; refusing BEFORE the fd moves",
            metadata.version
        );
    }

    let (fd, token) = recv_master_fd(stream).context("receiving master fd")?;
    let token = String::from_utf8_lossy(&token).into_owned();
    let expected = handoff_token(
        &metadata.runtime_key,
        metadata.shell_pid,
        metadata.shell_start_time,
    );
    if token != expected {
        // The fd is already ours; dropping `fd` closes it, which is the right
        // outcome for a descriptor we cannot identify.
        bail!("handoff token {token:?} does not match the metadata line ({expected:?})");
    }
    Ok((metadata, fd))
}

/// Read one `\n`-terminated line WITHOUT consuming a byte past it.
///
/// ⛔ **`BufReader` cannot be used here, and the reason is not performance.**
/// It reads ahead into its buffer, and the bytes after the metadata line are
/// the payload of the `sendmsg` that carries the descriptor. Consuming that
/// message with an ordinary `read` takes its data AND **closes the ancillary
/// descriptor** — the fd is destroyed in transit, and the following
/// `recvmsg` then blocks forever waiting for a message the kernel already
/// delivered.
///
/// Found by the round-trip test HANGING rather than failing, which is the
/// honest symptom: a lost fd is not an error the receiver can observe. One
/// byte at a time is the correct amount of clever here — the line is a few
/// hundred bytes and it runs once per session handoff.
fn read_line_without_overreading(stream: &UnixStream) -> std::io::Result<String> {
    use std::io::Read;
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match (&mut { stream }).read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                line.push(byte[0]);
                if line.len() > MAX_HANDOFF_METADATA_BYTES {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "handoff metadata line exceeded {MAX_HANDOFF_METADATA_BYTES} bytes \
                             with no newline"
                        ),
                    ));
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

/// A screen can be large, but not unbounded: a peer that never sends a newline
/// must not grow this reader without limit.
const MAX_HANDOFF_METADATA_BYTES: usize = 8 * 1024 * 1024;

/// Write the ack the predecessor is waiting on.
pub(crate) fn send_ack(stream: &UnixStream, ack: &HandoffAck) -> Result<()> {
    let mut line = serde_json::to_string(ack).context("encoding handoff ack")?;
    line.push('\n');
    (&mut { stream })
        .write_all(line.as_bytes())
        .context("writing handoff ack")?;
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn metadata() -> HandoffMetadata {
        HandoffMetadata {
            version: HANDOFF_WIRE_VERSION,
            runtime_key: "local://demo".to_string(),
            launch_command: "bash".to_string(),
            cwd: Some("/tmp".to_string()),
            cols: 80,
            rows: 24,
            shell_pid: 4242,
            shell_start_time: 999,
            screen: "hello\r\n".to_string(),
        }
    }

    /// The socket is named for the version, so a predecessor that knows only
    /// the successor's version string can find it without a discovery step.
    #[test]
    fn the_handoff_socket_is_derived_from_the_successor_version() {
        let path = handoff_socket_path(Path::new("/home/user/.yggterm"), "3.0.30");
        assert_eq!(
            path,
            Path::new("/home/user/.yggterm/pty-handoff-3-0-30.sock"),
            "dots become dashes exactly as the request socket does"
        );
    }

    /// A round trip over a real socketpair: metadata, then the descriptor.
    #[test]
    fn a_handoff_carries_the_metadata_and_then_the_descriptor() {
        let (a, b) = UnixStream::pair().expect("socketpair");
        let (carried, mut far_side) = UnixStream::pair().expect("carried pair");
        let meta = metadata();

        // Sender half, written out rather than calling send_session, because
        // that function also blocks on an ack this test answers below.
        let mut line = serde_json::to_string(&meta).unwrap();
        line.push('\n');
        (&a).write_all(line.as_bytes()).unwrap();
        send_master_fd(
            &a,
            std::os::fd::AsRawFd::as_raw_fd(&carried),
            handoff_token(&meta.runtime_key, meta.shell_pid, meta.shell_start_time).as_bytes(),
        )
        .unwrap();
        drop(carried);

        let (got_meta, fd) = receive_session(&b).expect("receive_session");
        assert_eq!(got_meta, meta, "the screen must survive the trip");

        // And it is the SAME descriptor, not merely a valid one.
        use std::io::Read;
        far_side.write_all(b"ACROSS").unwrap();
        drop(far_side);
        let mut got = String::new();
        UnixStream::from(fd).read_to_string(&mut got).unwrap();
        assert_eq!(got, "ACROSS");
    }

    /// An unknown wire version must be refused BEFORE the descriptor moves —
    /// the only point at which a refusal costs nothing.
    #[test]
    fn an_unknown_wire_version_is_refused_before_the_fd_moves() {
        let (a, b) = UnixStream::pair().expect("socketpair");
        let mut meta = metadata();
        meta.version = HANDOFF_WIRE_VERSION + 7;
        let mut line = serde_json::to_string(&meta).unwrap();
        line.push('\n');
        (&a).write_all(line.as_bytes()).unwrap();

        let error = receive_session(&b).expect_err("a future wire version must be refused");
        let text = format!("{error:#}");
        assert!(
            text.contains("BEFORE the fd moves"),
            "the refusal must say the fd did not move, got: {text}"
        );
    }

    /// A descriptor whose token disagrees with the metadata must not be seated.
    /// Getting this wrong means a PTY installed against the wrong session
    /// record — the user's shell answering under someone else's row.
    #[test]
    fn a_token_that_disagrees_with_the_metadata_is_refused() {
        let (a, b) = UnixStream::pair().expect("socketpair");
        let (carried, _far) = UnixStream::pair().expect("carried pair");
        let meta = metadata();
        let mut line = serde_json::to_string(&meta).unwrap();
        line.push('\n');
        (&a).write_all(line.as_bytes()).unwrap();
        send_master_fd(
            &a,
            std::os::fd::AsRawFd::as_raw_fd(&carried),
            handoff_token("local://someone-else", 1, 2).as_bytes(),
        )
        .unwrap();

        let error = receive_session(&b).expect_err("a mismatched token must be refused");
        assert!(format!("{error:#}").contains("does not match the metadata line"));
    }

    /// The commit flag is the whole contract of a failed handoff, so it is
    /// stated in the message a human will read in a trace.
    #[test]
    fn a_handoff_error_says_whether_the_fd_survived() {
        let before = HandoffError {
            committed: false,
            message: "connect failed".to_string(),
        };
        let after = HandoffError {
            committed: true,
            message: "no ack".to_string(),
        };
        assert!(before.to_string().contains("the fd stayed"));
        assert!(after.to_string().contains("the fd is gone"));
    }

    /// The token must fit the wire's own limit — the screen goes in the line.
    #[test]
    fn the_token_stays_inside_the_wire_limit() {
        let token = handoff_token("remote-cc://dev/2289eb57-c363-4331-8801-f2d10c6514f9", 4194303, u64::MAX);
        assert!(
            token.len() <= HANDOFF_TOKEN_MAX_BYTES,
            "token is {} bytes, over the wire limit",
            token.len()
        );
    }
}
