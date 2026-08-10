//! `server daemon-bridge` — the door a PHONE comes through, and the reason it
//! is not a shell.
//!
//! ## Why this exists (ADR-0002 §4.1, the most serious finding of that review)
//!
//! The phone reaches its fleet over SSH, because [[spec-decentralized-host-daemon]]
//! already settled that *"SSH is transport AND auth"*. The naive way to enrol a
//! phone is to append its public key to `~/.ssh/authorized_keys`. **That grants a
//! full interactive shell as the user** — and on `dev` that user owns
//! `~/.yggterm-keys/android-release.jks`, the *permanent* Android signing key.
//!
//! ⇒ A stolen phone would not be a lost session. It would be a **supply-chain
//! compromise of every yggterm install on earth**, because whoever holds that
//! keystore can sign an update that Android and Obtainium accept as ours.
//!
//! So the phone's key must never be a bare key. It is enrolled as a **forced
//! command** pointing here:
//!
//! ```text
//! restrict,command="~/.yggterm/bin/yggterm-headless server daemon-bridge" ssh-ed25519 AAAA... phone
//! ```
//!
//! `restrict` (OpenSSH 7.2+) turns off port/agent/X11 forwarding and pty
//! allocation; `command=` replaces whatever the client asked to run. The phone
//! therefore gets **the daemon protocol and nothing else** — no shell, no
//! `scp`, no tunnel back to the LAN.
//!
//! ## What it does
//!
//! Newline-delimited JSON in on stdin, newline-delimited JSON out on stdout, one
//! response per request, in order. Each request opens its own connection to the
//! daemon, because that is the daemon's own model — `handle_unix_stream` reads
//! ONE request, writes one response and returns, and there is no connection
//! reuse anywhere in it. The bridge amortises that for the phone: one SSH
//! channel, many requests.
//!
//! ## ⛔ It is a RAW LINE PROXY, and that is deliberate
//!
//! It does not deserialize `ServerRequest`. It forwards bytes.
//!
//! The daemon's wire carries **no version field and no capability negotiation**
//! — compatibility rests entirely on `#[serde(default)]` and the absence of
//! `deny_unknown_fields`. A phone ships on a store review cycle and will meet
//! daemons both older and newer than the release it was built against. If this
//! bridge parsed the enum, it would become a **third** thing that must know
//! every variant, and the oldest of the three would silently gate the newest.
//! Forwarding bytes means the bridge never needs a new release when a verb is
//! added, and the phone talks to the daemon it actually reached.
//!
//! ## ⚠ The honest limit — this is a SHELL DENIAL, not a sandbox
//!
//! Some `ServerRequest` variants legitimately start processes (a terminal ensure
//! spawns a PTY; a session launch runs its launch command). A key restricted to
//! this bridge therefore still commands real work on the host — it simply cannot
//! ask for an interactive shell, forward a port, or read an arbitrary file.
//!
//! That is a large reduction and it is the one worth having first, but **do not
//! describe this as a sandbox.** Narrowing *which* requests a given device may
//! send is a separate, later piece of work, and it must land in the daemon:
//! ADR-0002 §4.2 records that `role_gate` cannot do it today, because
//! `client_role` is a field the *sender* supplies with a permissive default.

use std::io::{BufRead, BufReader, Write};

use anyhow::{Context, Result};

use crate::daemon::{ServerEndpoint, resolve_client_daemon_endpoint};
use yggterm_core::resolve_yggterm_home;

/// How long a single proxied request may take before the bridge gives up on it
/// and answers with an error line.
///
/// Deliberately generous: a request that legitimately spawns a PTY is slow, and
/// a phone on a train is slower. The cost of being too eager here is an error
/// line for a request that would have succeeded, which the phone cannot tell
/// apart from a real failure.
const BRIDGE_REQUEST_TIMEOUT_MS: u64 = 30_000;

/// Answer a request the bridge could not deliver, **as a protocol line rather
/// than by dying**.
///
/// ⛔ The process must survive a bad request. One unparseable line, or one
/// daemon that went away mid-session, must not tear down the SSH channel and
/// every queued request behind it — a phone would see that as "the fleet is
/// gone" and there is no way for it to tell the difference.
fn bridge_error_line(message: &str) -> String {
    serde_json::json!({
        "kind": "bridge_error",
        "error": message,
    })
    .to_string()
}

/// Forward one already-framed request line to `endpoint` and return the
/// daemon's response line verbatim.
fn proxy_one(endpoint: &ServerEndpoint, request_line: &str) -> Result<String> {
    let mut bytes = request_line.trim_end().as_bytes().to_vec();
    bytes.push(b'\n');
    let io_timeout = Some(std::time::Duration::from_millis(BRIDGE_REQUEST_TIMEOUT_MS));

    match endpoint {
        #[cfg(unix)]
        ServerEndpoint::UnixSocket(path) => {
            let mut stream = std::os::unix::net::UnixStream::connect(path)
                .with_context(|| format!("connecting to {}", path.display()))?;
            stream.set_read_timeout(io_timeout).ok();
            stream.set_write_timeout(io_timeout).ok();
            stream.write_all(&bytes).context("writing bridged request")?;
            stream.flush().context("flushing bridged request")?;
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .context("reading bridged response")?;
            Ok(line)
        }
        ServerEndpoint::Tcp { host, port } => {
            let mut stream = std::net::TcpStream::connect((host.as_str(), *port))
                .with_context(|| format!("connecting to {host}:{port}"))?;
            stream.set_read_timeout(io_timeout).ok();
            stream.set_write_timeout(io_timeout).ok();
            stream.write_all(&bytes).context("writing bridged request")?;
            stream.flush().context("flushing bridged request")?;
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .context("reading bridged response")?;
            Ok(line)
        }
    }
}

/// Run the bridge until stdin closes.
///
/// Resolution uses [`resolve_client_daemon_endpoint`], **not** `default_endpoint`,
/// so a phone reaches a session held by an older coexisting daemon. That is the
/// constitution's stated case: *"a session owned by an OLDER daemon is still a
/// first-class row in the current GUI, and clicking it must WORK"* — and the
/// phone is just another client of the same rule.
pub fn run_daemon_bridge() -> Result<()> {
    let home = resolve_yggterm_home()?;
    let endpoint = resolve_client_daemon_endpoint(&home).endpoint;

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            // A read error on stdin is the channel dying, not a bad request.
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        let response = match proxy_one(&endpoint, &line) {
            Ok(response) if !response.trim().is_empty() => response,
            // An EMPTY response is the daemon's documented failure shape, not a
            // success: an unknown `kind` fails to deserialize inside
            // `read_request` and returns BEFORE `write_response` is reached, so
            // the socket simply closes. Left raw, the phone sees `Ok(0)` and
            // then a parse error on "" — indistinguishable from a crashed
            // daemon. Naming it here is the whole reason this arm exists.
            Ok(_) => {
                bridge_error_line(
                    "daemon closed the connection without answering — most likely an unknown \
                     request kind for this daemon version",
                ) + "\n"
            }
            Err(error) => bridge_error_line(&format!("{error:#}")) + "\n",
        };

        if stdout.write_all(response.as_bytes()).is_err() {
            break;
        }
        // ⛔ Flush every line. The phone is waiting on THIS response before it
        // sends the next request; a buffered reply is an apparent hang, and the
        // user reads a hang as "yggterm is broken".
        if stdout.flush().is_err() {
            break;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bridge_error_is_a_protocol_line_not_a_crash() {
        let line = bridge_error_line("daemon went away");
        let parsed: serde_json::Value =
            serde_json::from_str(&line).expect("a bridge error must itself be valid JSON — the \
                                                phone parses this stream with one decoder");
        assert_eq!(parsed["kind"], "bridge_error");
        assert_eq!(parsed["error"], "daemon went away");
    }

    /// ⛔ The forced-command contract, pinned at the source.
    ///
    /// This is the security boundary of the whole mobile lane: if this verb ever
    /// grows a path that execs a shell, spawns an arbitrary command, or reads a
    /// caller-supplied file, the `authorized_keys` restriction stops meaning
    /// anything and a stolen phone reaches the Android signing keystore on `dev`.
    ///
    /// A reviewer will not reliably notice that in a diff. This test will.
    #[test]
    fn the_bridge_never_grows_a_way_to_run_a_command() {
        let source = include_str!("daemon_bridge.rs");
        // Strip the doc comments and this test module: they legitimately discuss
        // shells and commands in prose, and matching prose would make the guard
        // fire on its own explanation.
        let code: String = source
            .lines()
            .filter(|line| {
                let t = line.trim_start();
                !t.starts_with("//") && !t.starts_with("///") && !t.starts_with("//!")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let code = match code.find("mod tests") {
            Some(at) => code[..at].to_string(),
            None => code,
        };

        for forbidden in [
            "Command::new",
            "std::process",
            "exec(",
            "File::open",
            "read_to_string",
        ] {
            assert!(
                !code.contains(forbidden),
                "daemon-bridge must stay a pure stdio<->socket proxy, but it now contains \
                 `{forbidden}`. This verb is the forced command behind a phone's ssh key; a way \
                 to run or read anything here defeats the authorized_keys restriction and puts \
                 the permanent Android signing keystore on dev within reach of a stolen phone. \
                 See ADR-0002 §4.1."
            );
        }
    }
}
