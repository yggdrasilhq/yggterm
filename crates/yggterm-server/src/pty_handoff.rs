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
//!     |  metadata line (JSON + screen)  ->  |   parse; evaluate the seat verdict
//!     |  <- verdict line (proceed/no)       |   PRE-COMMIT — refusing here is FREE
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
//! ## ⛔⛔ THE VERDICT LINE — a refusal evaluated AFTER the commit point
//!
//! The successor's seat check (*is a DIFFERENT live child already under this
//! key?*) used to run only once the descriptor had already crossed. The policy
//! is right and does not change; the MOMENT was wrong, and it is the one thing
//! the protocol could fix for free, because **the metadata line already carries
//! everything the decision needs** — `runtime_key`, `shell_pid` and
//! `shell_start_time` — and it arrives before the fd.
//!
//! Measured on the build host, twice in one night, same key both times:
//!
//! ```text
//! superseded_self_retire_sweep  Partial { moved: 10, reason: "…: successor took
//!   the fd and refused to seat it: … (AFTER the commit point — the fd is gone)" }
//! ```
//!
//! ⚠ **And "the fd is gone" is itself false, which is why this was mis-read for
//! so long.** [`crate::terminal::HandoffTakeout::master_fd`] is BORROWED: the
//! predecessor's runtime keeps its own master, `sendmsg` moves a DUPLICATE, and
//! a successor that refuses simply drops that duplicate. Nothing is orphaned.
//! The real cost is that the sweep books a failure, so it can never reach
//! `AllMoved`, so the predecessor can never retire — a daemon pinned for life
//! holding every session it owns. Verified on the live host: the pinned
//! predecessor was still up 28 h later holding 83 `/dev/ptmx` descriptors.
//!
//! ⇒ The verdict does not make a genuine conflict succeed. It makes the refusal
//! **free, honest and attributable**: nothing crosses, `committed` is `false`,
//! and the trace can count conflicts apart from transport failures.
//!
//! ## Why the wire version is NOT bumped for it
//!
//! ⛔ A bump would make every pre-existing successor refuse the whole handoff
//! (`receive_session` rejects any version but its own), so a fix for a rare
//! refusal would break every ordinary handover to an older peer. Version
//! coexistence is the constitution's promise, and both directions are real on a
//! fleet — a 3.1.12 daemon was the predecessor in both failures above while a
//! 3.1.36 one was the successor.
//!
//! The step is therefore **additive and asked-for**:
//!
//! - the predecessor sets `precommit_verdict` in the metadata line. An older
//!   successor ignores an unknown field (no `deny_unknown_fields`) and behaves
//!   exactly as it does today;
//! - the successor answers a verdict **only when asked**, so an older
//!   predecessor — which would read that line as its ack — never sees one;
//! - a newer predecessor waits only [`PRECOMMIT_VERDICT_TIMEOUT`] and, on a
//!   timeout, proceeds exactly as today. The timeout is paid ONCE per sweep,
//!   not once per session: [`PrecommitSupport`] remembers the answer.
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

use std::io::Write;

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
    /// Whether the sender will read a [`HandoffVerdict`] before it sends the
    /// descriptor.
    ///
    /// ⛔ **This is what keeps the step safe against an OLDER predecessor.** A
    /// build that predates the verdict reads the first line after its `sendmsg`
    /// as its ack; a successor that answered unasked would hand it a line it
    /// cannot parse and turn a working handover into a reported failure. So the
    /// successor speaks only when this says someone is listening.
    ///
    /// `#[serde(default)]` is the other half: an older predecessor's line has no
    /// such field, deserialises to `false`, and gets today's behaviour exactly.
    #[serde(default)]
    pub precommit_verdict: bool,
}

/// The only wire version this build speaks.
pub(crate) const HANDOFF_WIRE_VERSION: u32 = 1;

/// The handoff listener's address, named for the daemon's version exactly like
/// the request socket, so a predecessor can derive it from a version string.
pub(crate) fn handoff_socket_path(home_dir: &Path, version: &str) -> PathBuf {
    home_dir.join(format!("pty-handoff-{}.sock", version.replace('.', "-")))
}

/// The inverse of [`handoff_socket_path`]'s file-name shape: parse
/// `pty-handoff-<major>-<minor>-<patch>.sock` back into its version triple.
/// Lives beside the builder so the name shape has exactly one owner — the
/// socket sweep classifies graveyard files by this parse ([11.61]'s census
/// never covered this name shape; 218 of them, some months old, stood on the
/// GUI host on 2026-09-06).
pub(crate) fn parse_handoff_socket_name(path: &Path) -> Option<(u64, u64, u64)> {
    let name = path.file_name()?.to_str()?;
    let rest = name.strip_prefix("pty-handoff-")?.strip_suffix(".sock")?;
    let mut parts = rest.split('-');
    let (Some(major), Some(minor), Some(patch)) = (parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    if parts.next().is_some() {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?, patch.parse().ok()?))
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
///
/// `support` is the sweep's memo of what THIS successor speaks, and it is the
/// reason an older peer costs one timeout for a whole retirement rather than
/// one per session. Pass the same value for every session of one sweep.
pub(crate) fn send_session(
    socket_path: &Path,
    metadata: &HandoffMetadata,
    master_fd: RawFd,
    support: &mut PrecommitSupport,
) -> std::result::Result<HandoffAck, HandoffError> {
    let mut stream = UnixStream::connect(socket_path).map_err(|error| HandoffError {
        committed: false,
        refused: false,
        message: format!(
            "connecting to successor handoff socket {}: {error}",
            socket_path.display()
        ),
    })?;
    // Short while we are only waiting to be told whether to send at all. The
    // ack timeout is far longer and is set below, once the successor has real
    // work to do.
    let _ = stream.set_read_timeout(Some(PRECOMMIT_VERDICT_TIMEOUT));

    let mut line = serde_json::to_string(metadata).map_err(|error| HandoffError {
        committed: false,
        refused: false,
        message: format!("encoding handoff metadata: {error}"),
    })?;
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|error| HandoffError {
            committed: false,
            refused: false,
            message: format!("sending handoff metadata: {error}"),
        })?;

    // ⭐ THE PRE-COMMIT VERDICT — the only place a refusal costs nothing.
    if metadata.precommit_verdict && *support != PrecommitSupport::Silent {
        read_precommit_verdict(&stream, support)?;
    }

    // ⛔ Never wait on a successor for ever. The caller parks every reader
    // before the first send, so a hung ack is not merely a slow retirement —
    // it is a host on which nobody is draining any pty. A timeout turns that
    // into a reported failure, which the sweep already knows how to survive.
    let _ = stream.set_read_timeout(Some(ACK_TIMEOUT));

    let token = handoff_token(
        &metadata.runtime_key,
        metadata.shell_pid,
        metadata.shell_start_time,
    );
    // THE COMMIT POINT.
    send_master_fd(&stream, master_fd, token.as_bytes()).map_err(|error| HandoffError {
        committed: false,
        refused: false,
        message: format!("sending master fd: {error}"),
    })?;

    // Past here every failure is a REPORT, not a recovery: the descriptor is
    // the successor's now whatever the ack says.
    let ack = read_ack_past_a_late_verdict(&stream)?;
    if !ack.adopted {
        return Err(HandoffError {
            committed: true,
            refused: true,
            message: format!(
                "successor took the fd and refused to seat it: {}",
                ack.error.unwrap_or_else(|| "no reason given".to_string())
            ),
        });
    }
    Ok(ack)
}

/// Wait for the successor's verdict, and learn from the silence.
///
/// Three outcomes, and each one is a different fact about the peer:
///
/// - a verdict arrives ⇒ the successor speaks this step; obey it;
/// - the read TIMES OUT ⇒ the successor predates the step. Record that once,
///   proceed exactly as before, and never pay the wait again this sweep;
/// - anything else — EOF, an unreadable line — ⇒ refuse BEFORE the commit
///   point. Sending a live shell into a peer we cannot parse is the one thing
///   this whole module exists to avoid, and refusing here costs nothing.
///
/// ⚠ Only a TIMEOUT marks the peer `Silent`. An EOF says the connection died,
/// which is a fact about this attempt and not about the successor's wire —
/// treating it as "old build" would disarm the step for every session after it.
fn read_precommit_verdict(
    stream: &UnixStream,
    support: &mut PrecommitSupport,
) -> std::result::Result<(), HandoffError> {
    let line = match read_line_without_overreading(stream) {
        Ok(line) => line,
        Err(error) if is_timeout(&error) => {
            *support = support.saw_silence();
            return Ok(());
        }
        Err(error) => {
            return Err(HandoffError {
                committed: false,
                refused: false,
                message: format!("reading the successor's pre-commit verdict: {error}"),
            });
        }
    };
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(HandoffError {
            committed: false,
            refused: false,
            message: "successor closed the handoff before answering whether it could seat \
                      the session"
                .to_string(),
        });
    }
    let verdict: HandoffVerdict = serde_json::from_str(trimmed).map_err(|error| HandoffError {
        committed: false,
        refused: false,
        message: format!("successor sent an unreadable pre-commit verdict ({error}): {trimmed:?}"),
    })?;
    *support = PrecommitSupport::Speaks;
    if verdict.proceed {
        return Ok(());
    }
    Err(HandoffError {
        committed: false,
        refused: true,
        message: format!(
            "successor refused to seat it: {}",
            verdict
                .error
                .unwrap_or_else(|| "no reason given".to_string())
        ),
    })
}

/// Read the ack, stepping over a verdict this send raced past.
///
/// ⛔ **The one desync this protocol can produce, closed here.** A predecessor
/// whose verdict read timed out sends the descriptor anyway — and a successor
/// that was merely SLOW then writes its verdict, which would arrive where the
/// ack was expected and turn a perfectly good handover into "unreadable ack".
/// [`HandoffVerdict::precommit`] is the discriminant that makes that line
/// recognisable rather than merely wrong, so it can be skipped.
fn read_ack_past_a_late_verdict(
    stream: &UnixStream,
) -> std::result::Result<HandoffAck, HandoffError> {
    let no_ack = || HandoffError {
        committed: true,
        refused: false,
        message: "successor accepted the fd but never acknowledged it".to_string(),
    };
    // At most one verdict can be in flight, so one extra line is the whole
    // budget; a peer that sends more is not one to keep reading from.
    for _ in 0..2 {
        let line = read_line_without_overreading(stream).map_err(|_| no_ack())?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Err(no_ack());
        }
        if line_is_a_precommit_verdict(trimmed) {
            continue;
        }
        return serde_json::from_str(trimmed).map_err(|error| HandoffError {
            committed: true,
            refused: false,
            message: format!("successor sent an unreadable ack ({error}): {trimmed:?}"),
        });
    }
    Err(HandoffError {
        committed: true,
        refused: false,
        message: "successor sent pre-commit verdicts but never an ack".to_string(),
    })
}

fn line_is_a_precommit_verdict(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|value| {
            value
                .get(PRECOMMIT_DISCRIMINANT)
                .and_then(serde_json::Value::as_bool)
        })
        .unwrap_or(false)
}

fn is_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// How long a predecessor waits for the successor's ack before calling the
/// handoff failed. Generous — the successor has to seat the pty and persist —
/// but finite, because every reader on the host is parked while this blocks.
const ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// How long a predecessor waits to be told whether it may send at all.
///
/// Much shorter than [`ACK_TIMEOUT`], because the successor has nothing to do
/// but read a line, take its own lock and answer — but not so short that a
/// loaded daemon holding its runtime lock reads as an old build. Paid at most
/// once per sweep; see [`PrecommitSupport`].
const PRECOMMIT_VERDICT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// The field that tells a verdict line apart from an ack line.
const PRECOMMIT_DISCRIMINANT: &str = "precommit";

/// What a predecessor has learned about ONE successor's wire, over one sweep.
///
/// ⛔ **Per sweep, never per session.** A retiring daemon hands over every
/// runtime it owns to the same successor, one connection each. Asking an old
/// build for a verdict costs [`PRECOMMIT_VERDICT_TIMEOUT`]; asking it thirty
/// times costs a minute and a half of a daemon that is holding the host's PTYs.
/// Learning it once is the difference between an additive step and a tax.
///
/// ⛔⛔ **AND ONE SILENCE IS NOT AN ANSWER — IT TAKES TWO.** A successor answers
/// from behind its own runtime lock, and that lock is contended precisely
/// during a handover, which is the only time this code runs. On a single-strike
/// memo one contended moment would disarm the step for every remaining session
/// of the sweep — silently reverting to the behaviour being fixed — *and* stamp
/// `precommit: "silent"` on the trace, which a reader would take to mean the
/// successor is an old build. That is an instrument answering a different
/// question from its name, and it is the failure this whole module keeps
/// catching in other people's code.
///
/// A genuinely old successor is silent on every session, so two strikes cost it
/// one extra wait per sweep and nothing else. `Speaks` is never revoked: a peer
/// that has answered once is known to answer, and a straggling verdict from it
/// is stepped over by [`read_ack_past_a_late_verdict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PrecommitSupport {
    /// Nothing learned yet — wait for a verdict.
    #[default]
    Unknown,
    /// A verdict arrived. This successor will send one again.
    Speaks,
    /// One unanswered wait. An old build and a busy new one look identical
    /// here, so this decides nothing on its own.
    OneSilence,
    /// Silent twice. Do not wait again this sweep.
    Silent,
}

impl PrecommitSupport {
    /// For the trace: what the sweep found out about its successor.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Speaks => "speaks",
            Self::OneSilence => "one_silence",
            Self::Silent => "silent",
        }
    }

    /// A wait that went unanswered. Only the SECOND one concludes anything.
    fn saw_silence(self) -> Self {
        match self {
            // Never revoked: it has already proved it answers.
            Self::Speaks => Self::Speaks,
            Self::Unknown => Self::OneSilence,
            Self::OneSilence | Self::Silent => Self::Silent,
        }
    }
}

/// The successor's answer to *may I send it?*, given BEFORE the fd moves.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct HandoffVerdict {
    /// Always `true`. Not decoration: it is what lets a predecessor recognise
    /// this line if it arrives where an ack was expected — see
    /// [`read_ack_past_a_late_verdict`].
    pub precommit: bool,
    /// `true` — send the descriptor. `false` — keep it; nothing has moved.
    pub proceed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl HandoffVerdict {
    pub(crate) fn proceed() -> Self {
        Self {
            precommit: true,
            proceed: true,
            error: None,
        }
    }

    pub(crate) fn refused(error: String) -> Self {
        Self {
            precommit: true,
            proceed: false,
            error: Some(error),
        }
    }
}

/// Write the verdict the predecessor is waiting on.
///
/// ⛔ Call this ONLY when [`HandoffMetadata::precommit_verdict`] says the
/// predecessor is listening. An unasked verdict lands where an older build
/// expects its ack.
pub(crate) fn send_verdict(stream: &UnixStream, verdict: &HandoffVerdict) -> Result<()> {
    let mut line = serde_json::to_string(verdict).context("encoding handoff verdict")?;
    line.push('\n');
    (&mut { stream })
        .write_all(line.as_bytes())
        .context("writing handoff verdict")?;
    Ok(())
}

/// Whether a failed handoff left the descriptor behind or took it.
#[derive(Debug, Clone)]
pub(crate) struct HandoffError {
    /// `false` — nothing moved, the caller still owns its PTY and may retry.
    /// `true` — the fd is gone and the session is the successor's; the caller
    /// must NOT keep driving it and must NOT re-send.
    pub committed: bool,
    /// The successor ANSWERED, and its answer was no.
    ///
    /// ⛔ **Not the same question as `!committed`, and conflating them makes a
    /// counter lie.** A connect that failed and a socket that hung up are also
    /// uncommitted, but they say something about the WIRE; a refusal says
    /// something about the successor's own sessions. The pair is a 2×2 worth
    /// reading as one: refused-before-commit is free and expected, refused
    /// AFTER it is the defect this protocol step exists to drive to zero, and
    /// the two failure quadrants are transport problems wearing neither name.
    pub refused: bool,
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
    /// Who is holding it now — so the predecessor can wait for this successor
    /// to *survive* rather than merely to *accept*, and can tell it apart from
    /// any other daemon that later answers to the same version name.
    ///
    /// ⛔ **A bare pid is not an identity** (the rule this crate already applies
    /// to adopted children): pid plus start time, or nothing. Optional in both
    /// directions, so a build that predates it simply does not name itself and
    /// the predecessor falls back to asking the socket.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adopter_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adopter_start_time: Option<u64>,
}

impl HandoffAck {
    /// This daemon, named the way [`crate::pty_adoption`] names a process.
    pub(crate) fn adopted_here() -> Self {
        let pid = std::process::id();
        Self {
            adopted: true,
            error: None,
            adopter_pid: Some(pid),
            adopter_start_time: crate::pty_adoption::process_start_time(pid),
        }
    }

    pub(crate) fn refused(error: String) -> Self {
        Self {
            adopted: false,
            error: Some(error),
            adopter_pid: None,
            adopter_start_time: None,
        }
    }

    /// The `(pid, start_time)` pair, present only when both halves are.
    pub(crate) fn adopter_identity(&self) -> Option<(u32, u64)> {
        Some((self.adopter_pid?, self.adopter_start_time?))
    }
}

/// Read one handoff from an accepted connection: the metadata line, then the
/// descriptor.
///
/// Refuses an unknown wire version and a token that disagrees with the line —
/// both BEFORE returning, so a mismatched descriptor is closed by `OwnedFd`'s
/// drop rather than seated against the wrong record.
///
/// ⚠ The daemon does NOT call this: it needs the gap between the two halves,
/// because that gap is where a refusal is still free. It is the composition of
/// [`receive_metadata`] and [`receive_descriptor`], kept so the round-trip
/// tests exercise the pair exactly as the pieces are ordered on the wire.
pub(crate) fn receive_session(stream: &UnixStream) -> Result<(HandoffMetadata, OwnedFd)> {
    let metadata = receive_metadata(stream)?;
    let fd = receive_descriptor(stream, &metadata)?;
    Ok((metadata, fd))
}

/// Read the metadata line — everything that arrives BEFORE the commit point.
///
/// This is the half that makes the pre-commit verdict possible at all: it
/// carries `runtime_key`, `shell_pid` and `shell_start_time`, which is the
/// entire input to the seat decision, and it costs the predecessor nothing to
/// have sent.
pub(crate) fn receive_metadata(stream: &UnixStream) -> Result<HandoffMetadata> {
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
    Ok(metadata)
}

/// Take the descriptor, and refuse one whose token disagrees with the line.
///
/// ⛔ Calling this IS the commit point from the receiver's side: after it the
/// caller owns a duplicate of the predecessor's master and every refusal past
/// here is a report rather than a recovery.
pub(crate) fn receive_descriptor(
    stream: &UnixStream,
    metadata: &HandoffMetadata,
) -> Result<OwnedFd> {
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
    Ok(fd)
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

/// Serve ONE handoff connection, start to finish, in the order the wire
/// requires — the successor half, owned in one place.
///
/// ⛔⛔ **THE ORDER IS THE CONTRACT, SO ONLY ONE FUNCTION MAY KNOW IT.** Line,
/// then verdict, then descriptor, then ack: every one of those steps is
/// conditional on the last, and the whole defect this exists to fix was a
/// decision taken one step too late. A daemon that re-assembled the sequence
/// inline and a test that re-assembled it again would be two encodings of one
/// contract, and the test would then be proving its own copy.
///
/// The two callbacks are the only things that differ between a real daemon and
/// a test: `seat` answers whether this key may be seated (`Some(reason)` is a
/// refusal), and `adopt` installs the descriptor. Both are called with the
/// runtime lock held by the caller and neither may block on the socket.
pub(crate) fn serve_handoff(
    stream: &UnixStream,
    seat: &mut dyn FnMut(&HandoffMetadata) -> Option<String>,
    adopt: &mut dyn FnMut(&HandoffMetadata, OwnedFd) -> std::result::Result<(), String>,
) -> HandoffServed {
    // ⛔ EVERY refusal answers, including the ones that never reach a
    // descriptor. A predecessor that is told nothing waits out the full ack
    // timeout with all its readers parked, so silence costs the host ten
    // seconds per session to say what one line says immediately.
    let refuse = |key: Option<String>, error: String, before_commit: bool| {
        let served = HandoffServed::refused(key, error, before_commit);
        let _ = send_ack(
            stream,
            &HandoffAck::refused(served.error.clone().unwrap_or_default()),
        );
        served
    };

    let metadata = match receive_metadata(stream) {
        Ok(metadata) => metadata,
        // Nothing was read that could name a session, and no descriptor moved.
        Err(error) => return refuse(None, format!("{error:#}"), true),
    };
    let key = Some(metadata.runtime_key.clone());

    // ⭐ THE SEAT DECISION, TAKEN WHILE THE DESCRIPTOR IS STILL THE
    // PREDECESSOR'S. Everything it needs arrived in the line above.
    let conflict = seat(&metadata);
    if metadata.precommit_verdict {
        let verdict = match &conflict {
            Some(reason) => HandoffVerdict::refused(reason.clone()),
            None => HandoffVerdict::proceed(),
        };
        if let Err(error) = send_verdict(stream, &verdict) {
            return refuse(key, format!("{error:#}"), true);
        }
    }
    if let Some(reason) = conflict {
        // ⛔ DO NOT `recvmsg`. Leaving the descriptor in the socket costs
        // nothing: the kernel discards it with the connection, and the
        // predecessor's own master — the one the session actually runs on — is
        // untouched. Taking it only to drop it is what made every trace this
        // refusal produced say "the fd is gone" about a descriptor that never
        // went anywhere.
        return refuse(key, reason, true);
    }

    let fd = match receive_descriptor(stream, &metadata) {
        Ok(fd) => fd,
        Err(error) => {
            // ⛔ NOT `before_commit`. `receive_descriptor` refuses a token that
            // disagrees with the line, and by then the `recvmsg` has already
            // happened — the descriptor crossed and is dropped on the way out.
            // Booking that as free would make the counter answer a different
            // question from its name, which is the whole family of defect this
            // module keeps catching.
            return refuse(key, format!("{error:#}"), false);
        }
    };

    // PAST THE COMMIT POINT. Every refusal from here is a report.
    let served = match adopt(&metadata, fd) {
        Ok(()) => HandoffServed {
            runtime_key: key,
            adopted: true,
            error: None,
            refused_before_commit: false,
        },
        Err(error) => HandoffServed {
            runtime_key: key,
            adopted: false,
            error: Some(error),
            refused_before_commit: false,
        },
    };
    let ack = if served.adopted {
        HandoffAck::adopted_here()
    } else {
        HandoffAck::refused(
            served
                .error
                .clone()
                .unwrap_or_else(|| "no reason given".to_string()),
        )
    };
    let _ = send_ack(stream, &ack);
    served
}

/// What [`serve_handoff`] did with one connection.
#[derive(Debug, Clone)]
pub(crate) struct HandoffServed {
    /// `None` only when the metadata line itself could not be read, which is
    /// the one failure that cannot name a session.
    pub runtime_key: Option<String>,
    pub adopted: bool,
    pub error: Option<String>,
    /// Whether the refusal landed on the free side of the commit point. ⚠ A
    /// reader counting `false` here is counting descriptors that crossed only
    /// to be closed — the thing this whole step exists to drive to zero.
    pub refused_before_commit: bool,
}

impl HandoffServed {
    fn refused(runtime_key: Option<String>, error: String, before_commit: bool) -> Self {
        Self {
            runtime_key,
            adopted: false,
            error: Some(error),
            refused_before_commit: before_commit,
        }
    }
}

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
            precommit_verdict: true,
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

    #[test]
    fn the_handoff_socket_name_parses_back_to_its_version() {
        let built = handoff_socket_path(Path::new("/home/user/.yggterm"), "3.2.71");
        assert_eq!(
            parse_handoff_socket_name(&built),
            Some((3, 2, 71)),
            "the sweep's graveyard classifier must read the builder's own shape"
        );
        for name in [
            "pty-handoff-3-2.sock",      // short version
            "pty-handoff-3-2-71-9.sock", // over-long version
            "pty-handoff-x-y-z.sock",    // not numbers
            "pty-handoff-3-2-71",        // no suffix
            "server-3-2-71.sock",        // another plane's name
        ] {
            assert_eq!(
                parse_handoff_socket_name(Path::new(name)),
                None,
                "{name} is not a handoff socket name"
            );
        }
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
            refused: false,
            message: "connect failed".to_string(),
        };
        let after = HandoffError {
            committed: true,
            refused: false,
            message: "no ack".to_string(),
        };
        assert!(before.to_string().contains("the fd stayed"));
        assert!(after.to_string().contains("the fd is gone"));
    }

    /// ⛔⛔ THE COMPATIBILITY MECHANISM ITSELF, LOCKED.
    ///
    /// An older predecessor's metadata line has never heard of
    /// `precommit_verdict`. If that ever stops defaulting to `false` — a
    /// `deny_unknown_fields`, a rename, a required field — the successor starts
    /// volunteering a verdict into a socket where an older build expects its
    /// ack, and every handover to that build begins reporting "unreadable ack".
    ///
    /// ⚠ The failure would be SILENT in exactly the direction nobody tests:
    /// this build talking to itself is fine, and only a fleet running two
    /// versions at once would ever see it.
    #[test]
    fn an_older_predecessors_line_is_still_readable_and_asks_for_nothing() {
        // Every field this build knows about EXCEPT the new one — literally
        // what a build that predates it puts on the wire.
        let older = serde_json::json!({
            "version": HANDOFF_WIRE_VERSION,
            "runtime_key": "local://demo",
            "launch_command": "bash",
            "cwd": "/tmp",
            "cols": 80,
            "rows": 24,
            "shell_pid": 4242,
            "shell_start_time": 999,
            "screen": "hello\r\n",
        })
        .to_string();

        let decoded: HandoffMetadata = serde_json::from_str(&older)
            .expect("a line from a build that predates the verdict must still decode");
        assert!(
            !decoded.precommit_verdict,
            "an older predecessor asks for nothing, so the successor must say \
             nothing before the fd — it reads the next line as its ack"
        );

        // And the reverse direction: our line carries fields an older successor
        // has never seen, and must not be rejected for it.
        let ours = serde_json::to_string(&metadata()).unwrap();
        assert!(
            ours.contains("precommit_verdict"),
            "this build must ANNOUNCE that it will read a verdict, or the \
             successor stays silent and the step never fires"
        );
    }

    /// ⛔ THE ONE DESYNC THIS PROTOCOL CAN PRODUCE, AND THE GUARD FOR IT.
    ///
    /// A predecessor whose verdict read timed out sends the descriptor anyway.
    /// A successor that was merely SLOW — a loaded daemon holding its runtime
    /// lock — then writes its verdict, and that line lands exactly where the
    /// ack was expected. Without the discriminant it reads as an unparseable
    /// ack, and a handover that would have succeeded is booked as a failure.
    #[test]
    fn a_verdict_that_arrives_late_is_stepped_over_rather_than_read_as_the_ack() {
        let (a, b) = UnixStream::pair().expect("socketpair");
        // Both lines already in the buffer: the reader gave up waiting for the
        // first, so it meets them in this order.
        send_verdict(&a, &HandoffVerdict::proceed()).expect("verdict");
        send_ack(&a, &HandoffAck::adopted_here()).expect("ack");

        let ack = read_ack_past_a_late_verdict(&b)
            .expect("a late verdict must not be mistaken for an unreadable ack");
        assert!(ack.adopted);
        assert_eq!(ack.adopter_pid, Some(std::process::id()));
    }

    /// The negative control for the test above: without the discriminant the
    /// skip could match anything, so prove an ACK is never mistaken for a
    /// verdict and silently swallowed.
    #[test]
    fn an_ack_is_never_mistaken_for_a_verdict() {
        let ack = serde_json::to_string(&HandoffAck::adopted_here()).unwrap();
        assert!(!line_is_a_precommit_verdict(&ack));
        let refusal = serde_json::to_string(&HandoffAck::refused("busy".into())).unwrap();
        assert!(!line_is_a_precommit_verdict(&refusal));
        let verdict = serde_json::to_string(&HandoffVerdict::refused("busy".into())).unwrap();
        assert!(line_is_a_precommit_verdict(&verdict));
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
