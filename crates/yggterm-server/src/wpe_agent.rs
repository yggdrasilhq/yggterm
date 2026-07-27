//! Lane-A increment 3: the daemon's client for `yggterm-wpe-agent`.
//!
//! **The dependency is a SPAWNED BINARY, never a crate.** `yggterm-wpe` is
//! deliberately not a workspace member: building it needs libwpewebkit-2.0-dev,
//! libwpebackend-fdo-1.0-dev and libGLESv2 on the host. Linking it here would
//! make the WPE stack a prerequisite for building *anything* in this repo, on
//! every fleet machine — the back door the crate's own header refuses. So the
//! daemon speaks the agent's JSON-per-line protocol over a Unix socket and
//! knows nothing about WebKit. A host without the binary is answered
//! [`WpeOutcome::NotProvisioned`] — a typed, named answer, not an error cascade.
//!
//! ## What this module owns, and what it deliberately does not
//!
//! It owns **transport and supervision**. It does NOT own the verb vocabulary:
//! [`crate::wpe_agent`] never enumerates verbs in a type, never validates
//! params, and never reshapes an answer. The agent is the single source of
//! truth for what a verb means, so a verb added there works here the day it
//! lands, and an unknown verb is refused by the agent in its own words rather
//! than by a stale copy of its vocabulary living in the daemon. [`KNOWN_VERBS`]
//! exists for `--help` text only and says so.
//!
//! ## Supervision follows the child-split doctrine
//!
//! The daemon owns the process (we spawn it, so `waitpid` answers honestly —
//! contrast [`crate::pty_adoption`], where an adopted child can never report
//! *how* it exited). A `status` round trip is the liveness probe. And a death
//! is **surfaced, never repaired**:
//!
//! - an agent that dies is latched [`WpeOutcome::AgentDead`], naming its pid and
//!   exit status, on **every** subsequent verb;
//! - only an explicit `agent restart` clears the latch and spawns a successor.
//!
//! There is no auto-respawn, for the same reason the verb plane has no
//! auto-recovery of a crashed view: an invisible respawn loop is strictly worse
//! than a visible dead surface. A crash loop that silently reappears healthy is
//! a fault the operator can neither see nor time-box.
//!
//! ## One request in flight, and a timeout that recycles the connection
//!
//! The agent serves connections one at a time (single-threaded GLib main
//! context), so the client holds `&mut self` for a whole round trip — the
//! serialization is by ownership, not by a lock that could be forgotten.
//!
//! When a request times out, the connection is **dropped, not reused**. A late
//! answer is still queued in that socket, and reusing it would hand request N+1
//! the answer to request N — every subsequent answer off by one, each one
//! internally well-formed. The `id` echo is checked for the same reason: two
//! independent guards, because a mislabelled answer is worse than no answer.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The binary the daemon spawns. Resolved, never linked.
pub const AGENT_BINARY_NAME: &str = "yggterm-wpe-agent";

/// Environment override for the agent binary path (absolute).
pub const AGENT_BINARY_ENV: &str = "YGGTERM_WPE_AGENT";

/// Verb names, **for help text only**.
///
/// The agent owns the vocabulary; this list is not consulted on the request
/// path and an unknown verb travels to the agent and is refused in the agent's
/// own words. Keeping it out of the request path is what stops it becoming a
/// second, drifting encoding of the verb surface.
pub const KNOWN_VERBS: &[&str] = &[
    "ensure",
    "navigate",
    "eval",
    "click",
    "type",
    "read-back",
    "capture-view",
    "capture-element",
    "restart",
    "status",
];

/// Param keys that must cross the wire as JSON numbers rather than strings.
///
/// This is the CLI's spelling problem, not a verb table: a command line has
/// only strings, and `{"width":"800"}` is not `{"width":800}` to the agent's
/// `as_u32`. Everything not listed stays a string, which is what every other
/// param already is.
pub const NUMERIC_PARAM_KEYS: &[&str] = &["width", "height", "timeout_ms"];

/// The agent's own default verb deadline (mirrors `AgentState::timeout`).
const DEFAULT_VERB_TIMEOUT_MS: u64 = 30_000;

/// How much longer than the agent's own deadline this client waits.
///
/// The client must never give up FIRST on a request the agent is still
/// honestly working: a timeout raised at exactly the agent's deadline is a coin
/// flip between "the agent timed out and said so" and "we stopped listening".
const CLIENT_GRACE_MS: u64 = 5_000;

/// Budget for a spawned agent to bind its socket and accept.
///
/// Generous because bring-up is a real EGL/WebKit initialisation, not a bind.
const SPAWN_READY_TIMEOUT_MS: u64 = 20_000;

/// After EOF mid-request, how long to wait for the child to be reapable.
///
/// A process that has closed its socket has not necessarily been scheduled
/// through exit yet, so `try_wait` can legitimately answer `None` for a moment.
/// Waiting bounded turns "the agent vanished" into "the agent exited with 17".
const DEATH_CONFIRM_MS: u64 = 1_000;

/// How much of the agent's stderr log to quote when it fails to start.
const LOG_TAIL_BYTES: usize = 600;

/// What the plane answered. Every arm is a NAMED failure mode, because the
/// caller's next move differs per arm: provision a binary, restart the agent,
/// retry, or fix the request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WpeOutcome {
    /// The agent answered `ok:true`. `response` is its object verbatim, minus
    /// nothing: the daemon does not reshape a verb's result.
    Answer { response: Value },
    /// The agent answered `ok:false`. Its `error` string, unedited.
    VerbFailed { message: String },
    /// No `yggterm-wpe-agent` binary could be resolved on this host. Not an
    /// error — the expected answer on every machine that has not provisioned
    /// the WPE stack, which is most of them.
    NotProvisioned { searched: String, detail: String },
    /// A resolved binary was spawned and exited before it could serve. Carries
    /// the agent's own honest startup diagnosis from its stderr log.
    StartFailed { exit: String, detail: String },
    /// The agent process is gone. Latched: every verb answers this until an
    /// explicit `agent restart`.
    AgentDead { pid: u32, exit: String },
    /// The agent did not answer within the deadline. The connection has been
    /// recycled, so the next verb gets its own answer.
    Timeout { verb: String, waited_ms: u64 },
    /// The socket itself failed, or the agent's answer was not usable.
    Transport { message: String },
}

impl WpeOutcome {
    /// Did the plane produce a verb answer? False for every failure arm.
    pub fn is_answer(&self) -> bool {
        matches!(self, WpeOutcome::Answer { .. })
    }

    /// A one-line human form, for CLI output and trace fields.
    pub fn summary(&self) -> String {
        match self {
            WpeOutcome::Answer { .. } => "ok".to_string(),
            WpeOutcome::VerbFailed { message } => format!("verb failed: {message}"),
            WpeOutcome::NotProvisioned { detail, .. } => {
                format!("no {AGENT_BINARY_NAME} on this host: {detail}")
            }
            WpeOutcome::StartFailed { exit, detail } => {
                format!("agent exited during start ({exit}): {detail}")
            }
            WpeOutcome::AgentDead { pid, exit } => {
                format!("agent pid {pid} is dead ({exit}); run `wpe agent restart`")
            }
            WpeOutcome::Timeout { verb, waited_ms } => {
                format!("{verb} did not answer within {waited_ms}ms")
            }
            WpeOutcome::Transport { message } => format!("transport: {message}"),
        }
    }
}

/// What supervision knows about the agent process. Deliberately flat and
/// printable: this is what an operator reads when a verb answered `AgentDead`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WpeAgentReport {
    /// `not_spawned` | `running` | `dead`.
    pub state: String,
    /// The resolved binary, or `None` when the host has none.
    pub binary: Option<String>,
    /// Why no binary could be resolved. Present only when `binary` is `None`.
    pub provisioning_detail: Option<String>,
    pub socket: String,
    /// Where the agent's stderr goes — the first thing to read on `StartFailed`.
    pub log: String,
    pub pid: Option<u32>,
    pub spawned_at_ms: Option<u64>,
    /// The exit status of the last agent that died, retained across the death
    /// latch so `agent status` can answer *how* it died, not merely *that*.
    pub last_exit: Option<String>,
    pub last_exit_pid: Option<u32>,
}

/// A running agent process the daemon spawned.
struct AgentProcess {
    child: Child,
    pid: u32,
    spawned_at_ms: u64,
}

/// A death that has not been acknowledged by an explicit `agent restart`.
#[derive(Clone)]
struct DeadAgent {
    pid: u32,
    exit: String,
}

/// The daemon's one client for the Lane-A verb plane.
pub struct WpeAgentClient {
    /// An explicitly configured binary path. `None` means "probe each time".
    configured_binary: Option<PathBuf>,
    socket_path: PathBuf,
    log_path: PathBuf,
    process: Option<AgentProcess>,
    dead: Option<DeadAgent>,
    connection: Option<std::os::unix::net::UnixStream>,
    next_id: u64,
}

impl WpeAgentClient {
    /// `binary: None` probes ([`AGENT_BINARY_ENV`], then beside the running
    /// executable, then `PATH`) on every use — so provisioning the binary later
    /// starts working without a daemon restart.
    pub fn new(binary: Option<PathBuf>, socket_path: PathBuf) -> Self {
        let log_path = socket_path.with_extension("log");
        WpeAgentClient {
            configured_binary: binary,
            socket_path,
            log_path,
            process: None,
            dead: None,
            connection: None,
            next_id: 1,
        }
    }

    /// The daemon's per-process socket path.
    ///
    /// Keyed by pid, because version-coexisting daemons are the constitution's
    /// requirement: two daemons sharing one socket path would fight over the
    /// same agent, and the second one to bind would unlink the first's socket.
    pub fn default_socket_path() -> PathBuf {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        base.join(format!("yggterm-wpe-{}.sock", std::process::id()))
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Run one verb. `params` is passed to the agent verbatim; `id`, `verb` and
    /// the deadline are the only fields this client adds.
    pub fn verb(&mut self, verb: &str, params: Map<String, Value>) -> WpeOutcome {
        // The death latch is checked BEFORE anything can start a process. This
        // ordering is the whole no-auto-respawn guarantee: move it below
        // `ensure_started` and a dead agent silently becomes a new one.
        if let Some(dead) = &self.dead {
            return WpeOutcome::AgentDead {
                pid: dead.pid,
                exit: dead.exit.clone(),
            };
        }
        if let Err(outcome) = self.ensure_started() {
            return outcome;
        }
        let timeout_ms = verb_timeout_ms(&params);
        let id = self.next_id.to_string();
        self.next_id += 1;

        let mut request = params;
        request.insert("id".to_string(), Value::String(id.clone()));
        request.insert("verb".to_string(), Value::String(verb.to_string()));
        let mut line = match serde_json::to_string(&Value::Object(request)) {
            Ok(line) => line,
            Err(error) => {
                return WpeOutcome::Transport {
                    message: format!("serializing the {verb} request: {error}"),
                };
            }
        };
        line.push('\n');

        let deadline = Duration::from_millis(timeout_ms.saturating_add(CLIENT_GRACE_MS));
        match self.round_trip(&line, &id, deadline) {
            Ok(answer) => answer,
            Err(RoundTripError::Recycle(outcome)) => {
                self.connection = None;
                outcome
            }
        }
    }

    /// Supervision: what the daemon knows about the process right now.
    ///
    /// Reaps first, so a process that died while nothing was asking is reported
    /// dead rather than running. Never spawns: asking about the agent must not
    /// create one.
    pub fn report(&mut self) -> WpeAgentReport {
        self.reap_if_exited();
        let (binary, provisioning_detail) = match self.resolve_binary() {
            Ok(path) => (Some(path.display().to_string()), None),
            Err(detail) => (None, Some(detail)),
        };
        let state = if self.dead.is_some() {
            "dead"
        } else if self.process.is_some() {
            "running"
        } else {
            "not_spawned"
        };
        WpeAgentReport {
            state: state.to_string(),
            binary,
            provisioning_detail,
            socket: self.socket_path.display().to_string(),
            log: self.log_path.display().to_string(),
            pid: self.process.as_ref().map(|process| process.pid),
            spawned_at_ms: self.process.as_ref().map(|process| process.spawned_at_ms),
            last_exit: self.dead.as_ref().map(|dead| dead.exit.clone()),
            last_exit_pid: self.dead.as_ref().map(|dead| dead.pid),
        }
    }

    /// Explicit `agent restart` — the ONLY thing that clears a death latch.
    ///
    /// Spawns eagerly rather than leaving a lazy spawn to the next verb, so the
    /// answer to "restart" is whether the agent is up, not a promise.
    pub fn restart_agent(&mut self) -> Result<WpeAgentReport, WpeOutcome> {
        self.stop_agent();
        self.ensure_started()?;
        Ok(self.report())
    }

    /// Explicit `agent stop` — release the process.
    ///
    /// This clears the death latch too, returning the plane to `not_spawned`,
    /// from which the next verb lazily spawns. That is the honest reading:
    /// `stop` releases a process, it does not disable the plane. Only an
    /// UNEXPECTED death latches, because only an unexpected death is a fault
    /// somebody needs to see.
    pub fn stop_agent(&mut self) -> WpeAgentReport {
        self.connection = None;
        if let Some(mut process) = self.process.take() {
            let _ = process.child.kill();
            let _ = process.child.wait();
        }
        self.dead = None;
        let _ = std::fs::remove_file(&self.socket_path);
        self.report()
    }

    // ---- process lifecycle ------------------------------------------------

    fn ensure_started(&mut self) -> Result<(), WpeOutcome> {
        self.reap_if_exited();
        if let Some(dead) = &self.dead {
            return Err(WpeOutcome::AgentDead {
                pid: dead.pid,
                exit: dead.exit.clone(),
            });
        }
        if self.process.is_some() {
            return Ok(());
        }
        let binary = self
            .resolve_binary()
            .map_err(|detail| WpeOutcome::NotProvisioned {
                searched: AGENT_BINARY_NAME.to_string(),
                detail,
            })?;

        // A stale socket from a predecessor would let us connect to nothing.
        let _ = std::fs::remove_file(&self.socket_path);
        if let Some(parent) = self.socket_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // stderr goes to a FILE, not a pipe: a pipe nobody drains fills at 64K
        // and blocks the agent inside a write to stderr — a supervisor that
        // deadlocks the thing it supervises. The file is also what makes
        // `StartFailed` able to quote the agent's own diagnosis.
        let log = std::fs::File::create(&self.log_path).map_err(|error| WpeOutcome::Transport {
            message: format!("creating {}: {error}", self.log_path.display()),
        })?;
        let child = Command::new(&binary)
            .arg(&self.socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log))
            .spawn()
            .map_err(|error| WpeOutcome::NotProvisioned {
                searched: binary.display().to_string(),
                detail: format!("spawning {}: {error}", binary.display()),
            })?;
        let pid = child.id();
        self.process = Some(AgentProcess {
            child,
            pid,
            spawned_at_ms: crate::current_millis_u64(),
        });
        self.wait_until_serving()
    }

    /// Wait for the freshly spawned agent to bind and accept.
    ///
    /// Two ways out, and they mean different things: the socket answers
    /// (`Ok`), or the child exits first (`StartFailed`, quoting the log). Both
    /// beat a bare timeout, which would tell the operator nothing about a host
    /// that simply lacks libwpewebkit.
    fn wait_until_serving(&mut self) -> Result<(), WpeOutcome> {
        let deadline = Instant::now() + Duration::from_millis(SPAWN_READY_TIMEOUT_MS);
        loop {
            if let Some(exit) = self.child_exit_status() {
                let pid = self
                    .process
                    .as_ref()
                    .map(|process| process.pid)
                    .unwrap_or(0);
                self.process = None;
                // Latched as dead: a host whose WPE stack is missing would
                // otherwise re-spawn a doomed process on every single verb.
                self.dead = Some(DeadAgent {
                    pid,
                    exit: exit.clone(),
                });
                return Err(WpeOutcome::StartFailed {
                    exit,
                    detail: self.log_tail(),
                });
            }
            match std::os::unix::net::UnixStream::connect(&self.socket_path) {
                Ok(stream) => {
                    self.connection = Some(stream);
                    return Ok(());
                }
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) => {
                    let pid = self
                        .process
                        .as_ref()
                        .map(|process| process.pid)
                        .unwrap_or(0);
                    self.stop_agent();
                    return Err(WpeOutcome::Transport {
                        message: format!(
                            "agent pid {pid} did not accept on {} within {SPAWN_READY_TIMEOUT_MS}ms: {error}",
                            self.socket_path.display()
                        ),
                    });
                }
            }
        }
    }

    /// Reap a process that exited while nothing was asking, latching the death.
    fn reap_if_exited(&mut self) {
        if let Some(exit) = self.child_exit_status() {
            let pid = self
                .process
                .as_ref()
                .map(|process| process.pid)
                .unwrap_or(0);
            self.process = None;
            self.connection = None;
            self.dead = Some(DeadAgent { pid, exit });
        }
    }

    fn child_exit_status(&mut self) -> Option<String> {
        let process = self.process.as_mut()?;
        match process.child.try_wait() {
            Ok(Some(status)) => Some(describe_exit(&status)),
            Ok(None) => None,
            // `try_wait` failing means we cannot answer honestly about this
            // child any more; treating that as death is the safe direction,
            // because the alternative is reporting a process we cannot see as
            // healthy.
            Err(error) => Some(format!("unwaitable: {error}")),
        }
    }

    /// Confirm a suspected death, bounded. Returns the latched outcome if the
    /// agent really is gone.
    fn confirm_death(&mut self) -> Option<WpeOutcome> {
        let deadline = Instant::now() + Duration::from_millis(DEATH_CONFIRM_MS);
        loop {
            if let Some(exit) = self.child_exit_status() {
                let pid = self
                    .process
                    .as_ref()
                    .map(|process| process.pid)
                    .unwrap_or(0);
                self.process = None;
                self.dead = Some(DeadAgent {
                    pid,
                    exit: exit.clone(),
                });
                return Some(WpeOutcome::AgentDead { pid, exit });
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn log_tail(&self) -> String {
        let Ok(mut file) = std::fs::File::open(&self.log_path) else {
            return format!("(no log at {})", self.log_path.display());
        };
        let mut text = String::new();
        if file.read_to_string(&mut text).is_err() {
            return format!("(unreadable log at {})", self.log_path.display());
        }
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return format!("(the agent wrote nothing to {})", self.log_path.display());
        }
        let tail = if trimmed.len() > LOG_TAIL_BYTES {
            let start = trimmed.len() - LOG_TAIL_BYTES;
            let start = trimmed
                .char_indices()
                .map(|(index, _)| index)
                .find(|index| *index >= start)
                .unwrap_or(0);
            &trimmed[start..]
        } else {
            trimmed
        };
        tail.replace('\n', " | ")
    }

    // ---- one round trip ---------------------------------------------------

    fn round_trip(
        &mut self,
        line: &str,
        id: &str,
        deadline: Duration,
    ) -> Result<WpeOutcome, RoundTripError> {
        let verb_deadline_ms = deadline.as_millis() as u64;
        let stream = self.connect()?;
        if stream.set_read_timeout(Some(deadline)).is_err() {
            return Err(RoundTripError::Recycle(WpeOutcome::Transport {
                message: "could not arm the agent read deadline".to_string(),
            }));
        }
        let _ = stream.set_write_timeout(Some(deadline));
        let mut write_half = match stream.try_clone() {
            Ok(half) => half,
            Err(error) => {
                return Err(RoundTripError::Recycle(WpeOutcome::Transport {
                    message: format!("splitting the agent connection: {error}"),
                }));
            }
        };
        if let Err(error) = write_half
            .write_all(line.as_bytes())
            .and_then(|()| write_half.flush())
        {
            // A write failing usually means the peer is gone. Say which.
            if let Some(dead) = self.confirm_death() {
                return Err(RoundTripError::Recycle(dead));
            }
            return Err(RoundTripError::Recycle(WpeOutcome::Transport {
                message: format!("writing the request: {error}"),
            }));
        }

        let read_half = match stream.try_clone() {
            Ok(half) => half,
            Err(error) => {
                return Err(RoundTripError::Recycle(WpeOutcome::Transport {
                    message: format!("splitting the agent connection: {error}"),
                }));
            }
        };
        let mut reader = BufReader::new(read_half);
        let mut response = String::new();
        let started = Instant::now();
        match reader.read_line(&mut response) {
            // EOF: the agent hung up without answering.
            Ok(0) => {
                if let Some(dead) = self.confirm_death() {
                    Err(RoundTripError::Recycle(dead))
                } else {
                    Err(RoundTripError::Recycle(WpeOutcome::Transport {
                        message: "the agent closed the connection without answering".to_string(),
                    }))
                }
            }
            Ok(_) => self.classify(&response, id),
            Err(error) => {
                // A dead agent whose socket simply stopped producing bytes must
                // be reported as dead, not as a timeout: "it is taking a while"
                // and "it will never answer" are different instructions to the
                // caller.
                if let Some(dead) = self.confirm_death() {
                    return Err(RoundTripError::Recycle(dead));
                }
                if is_timeout(&error) {
                    Err(RoundTripError::Recycle(WpeOutcome::Timeout {
                        verb: verb_of(line),
                        waited_ms: started.elapsed().as_millis() as u64,
                    }))
                } else {
                    Err(RoundTripError::Recycle(WpeOutcome::Transport {
                        message: format!(
                            "reading the answer (deadline {verb_deadline_ms}ms): {error}"
                        ),
                    }))
                }
            }
        }
    }

    fn connect(&mut self) -> Result<&std::os::unix::net::UnixStream, RoundTripError> {
        if self.connection.is_none() {
            match std::os::unix::net::UnixStream::connect(&self.socket_path) {
                Ok(stream) => self.connection = Some(stream),
                Err(error) => {
                    if let Some(dead) = self.confirm_death() {
                        return Err(RoundTripError::Recycle(dead));
                    }
                    return Err(RoundTripError::Recycle(WpeOutcome::Transport {
                        message: format!("connecting to {}: {error}", self.socket_path.display()),
                    }));
                }
            }
        }
        Ok(self.connection.as_ref().expect("just connected"))
    }

    /// Turn one answer line into an outcome.
    ///
    /// The `id` echo is checked here and a mismatch is refused rather than
    /// returned: an answer belonging to an earlier request is not "close
    /// enough", it is a lie about what just happened.
    fn classify(&mut self, response: &str, id: &str) -> Result<WpeOutcome, RoundTripError> {
        let value: Value = match serde_json::from_str(response.trim_end()) {
            Ok(value) => value,
            Err(error) => {
                return Err(RoundTripError::Recycle(WpeOutcome::Transport {
                    message: format!("the agent's answer was not JSON: {error}"),
                }));
            }
        };
        let echoed = value.get("id").and_then(Value::as_str).unwrap_or("");
        if echoed != id {
            return Err(RoundTripError::Recycle(WpeOutcome::Transport {
                message: format!(
                    "answer id {echoed:?} does not match request id {id:?}; the connection is out \
                     of step and has been dropped"
                ),
            }));
        }
        match value.get("ok").and_then(Value::as_bool) {
            Some(true) => Ok(WpeOutcome::Answer { response: value }),
            Some(false) => Ok(WpeOutcome::VerbFailed {
                message: value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("the agent refused the verb without saying why")
                    .to_string(),
            }),
            None => Err(RoundTripError::Recycle(WpeOutcome::Transport {
                message: "the agent's answer carried no \"ok\"".to_string(),
            })),
        }
    }

    // ---- binary resolution ------------------------------------------------

    /// Probe order: configured path, `$YGGTERM_WPE_AGENT`, beside the running
    /// executable, then `PATH`. Re-run on every use, deliberately: a host that
    /// installs the agent later starts working without a daemon restart, and a
    /// negative answer costs a few `stat`s.
    fn resolve_binary(&self) -> Result<PathBuf, String> {
        let mut searched: Vec<String> = Vec::new();
        if let Some(configured) = &self.configured_binary {
            if is_executable_file(configured) {
                return Ok(configured.clone());
            }
            return Err(format!(
                "the configured agent path {} is not an executable file",
                configured.display()
            ));
        }
        if let Some(from_env) = std::env::var_os(AGENT_BINARY_ENV)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
        {
            if is_executable_file(&from_env) {
                return Ok(from_env);
            }
            return Err(format!(
                "${AGENT_BINARY_ENV} points at {}, which is not an executable file",
                from_env.display()
            ));
        }
        if let Ok(current) = std::env::current_exe()
            && let Some(dir) = current.parent()
        {
            let sibling = dir.join(AGENT_BINARY_NAME);
            if is_executable_file(&sibling) {
                return Ok(sibling);
            }
            searched.push(sibling.display().to_string());
        }
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                let candidate = dir.join(AGENT_BINARY_NAME);
                if is_executable_file(&candidate) {
                    return Ok(candidate);
                }
            }
            searched.push("$PATH".to_string());
        }
        Err(format!(
            "no {AGENT_BINARY_NAME} found (looked at: {}); build crates/yggterm-wpe on a host \
             with libwpewebkit-2.0-dev, or point ${AGENT_BINARY_ENV} at the binary",
            if searched.is_empty() {
                "nowhere — no PATH and no executable dir".to_string()
            } else {
                searched.join(", ")
            }
        ))
    }
}

/// The daemon's ONE agent plane.
///
/// A process-global is the accurate encoding, not a convenience: `Engine` is a
/// process singleton (libwpe's loader, the EGL display and the current GL
/// context are all per-process), so there is exactly one agent per daemon by
/// construction. The socket path is keyed by THIS daemon's pid, which is what
/// lets version-coexisting daemons each own their own agent without fighting
/// over one socket.
///
/// Poisoning is recovered rather than propagated: a panic inside one verb must
/// not take the plane out of service for the life of the daemon. The worst a
/// recovered lock carries is a stale connection, and the next round trip
/// recycles it.
pub fn plane_lock() -> std::sync::MutexGuard<'static, WpeAgentClient> {
    static PLANE: std::sync::OnceLock<std::sync::Mutex<WpeAgentClient>> =
        std::sync::OnceLock::new();
    PLANE
        .get_or_init(|| {
            std::sync::Mutex::new(WpeAgentClient::new(
                None,
                WpeAgentClient::default_socket_path(),
            ))
        })
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Drop for WpeAgentClient {
    /// The daemon owns the process, so the daemon buries it. Without this an
    /// agent outlives the daemon that spawned it, holding a WebKit process
    /// tree and a socket nobody will ever connect to again.
    fn drop(&mut self) {
        self.connection = None;
        if let Some(process) = &mut self.process {
            let _ = process.child.kill();
            let _ = process.child.wait();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

enum RoundTripError {
    /// The connection is no longer trustworthy: hand back this outcome and
    /// drop it, so the NEXT verb starts from a fresh socket.
    Recycle(WpeOutcome),
}

/// The deadline the agent will apply to this request, so the client can wait
/// strictly longer. One reader of `timeout_ms`, so the two deadlines cannot
/// drift into a race.
fn verb_timeout_ms(params: &Map<String, Value>) -> u64 {
    params
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_VERB_TIMEOUT_MS)
}

/// How long a CLIENT of the daemon must be willing to wait for a WPE verb.
///
/// The daemon cannot answer before the agent does, so the daemon's own client
/// must not give up first. Same arithmetic as the plane's, twice-graced: agent
/// deadline → plane deadline → daemon-client deadline.
pub fn client_io_timeout_ms(params: &Map<String, Value>) -> u64 {
    verb_timeout_ms(params)
        .saturating_add(CLIENT_GRACE_MS)
        .saturating_add(CLIENT_GRACE_MS)
}

/// How long a CLIENT of the daemon must wait for `wpe agent <action>`.
///
/// `restart` spawns an agent and waits out a real WebKit bring-up, which is
/// budgeted at [`SPAWN_READY_TIMEOUT_MS`] — comfortably past the daemon's 10s
/// default. Left on the default, a bring-up that took twelve seconds would
/// surface to the caller as "no answer from the daemon" for a restart that in
/// fact succeeded, which is the same false negative the hot-restart budget
/// exists to prevent. Derived from the spawn budget rather than pinned to a
/// constant so the two cannot drift apart.
pub fn agent_control_io_timeout_ms() -> u64 {
    SPAWN_READY_TIMEOUT_MS
        .saturating_add(CLIENT_GRACE_MS)
        .saturating_add(CLIENT_GRACE_MS)
}

fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn is_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// Name an exit the way `waitpid` saw it. Signals are named separately from
/// codes because "killed by SIGKILL" and "exited 9" are different events and
/// collapsing them loses the one fact that explains an OOM kill.
fn describe_exit(status: &std::process::ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("killed by signal {signal}");
        }
    }
    match status.code() {
        Some(code) => format!("exited {code}"),
        None => "exited with no status".to_string(),
    }
}

/// The verb name back out of a request line, for a timeout message that says
/// WHICH verb hung.
fn verb_of(line: &str) -> String {
    serde_json::from_str::<Value>(line.trim_end())
        .ok()
        .and_then(|value| {
            value
                .get("verb")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "verb".to_string())
}

/// Parse `--key value` pairs into the agent's param object.
///
/// Numbers are coerced for [`NUMERIC_PARAM_KEYS`] only; everything else is a
/// string, because a command line has no other type and guessing (is `1` the
/// number one or the id "1"?) would silently change what the agent sees.
pub fn params_from_flags(args: &[String]) -> Result<Map<String, Value>, String> {
    let mut params = Map::new();
    let mut index = 0usize;
    while index < args.len() {
        let flag = &args[index];
        let Some(key) = flag.strip_prefix("--") else {
            return Err(format!(
                "unexpected argument {flag:?}; WPE verb params are given as --key value"
            ));
        };
        let key = key.replace('-', "_");
        let Some(raw) = args.get(index + 1) else {
            return Err(format!("--{key} needs a value"));
        };
        let value = if NUMERIC_PARAM_KEYS.contains(&key.as_str()) {
            let number: u64 = raw
                .parse()
                .map_err(|_| format!("--{key} takes a number, got {raw:?}"))?;
            Value::Number(number.into())
        } else {
            Value::String(raw.clone())
        };
        params.insert(key, value);
        index += 2;
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verbs_client_deadline_is_strictly_longer_than_the_agents() {
        let mut params = Map::new();
        params.insert("timeout_ms".to_string(), Value::from(1_000u64));
        assert_eq!(verb_timeout_ms(&params), 1_000);
        assert!(
            client_io_timeout_ms(&params) > verb_timeout_ms(&params),
            "the daemon's client must not give up before the agent's own deadline, or a \
             timed-out verb is indistinguishable from a dropped answer",
        );
    }

    /// A restart that is still honestly bringing WebKit up must not be
    /// reported to the caller as a daemon that stopped answering.
    #[test]
    fn the_agent_control_budget_outlasts_a_real_bring_up() {
        assert!(
            agent_control_io_timeout_ms() > SPAWN_READY_TIMEOUT_MS,
            "a client that gives up at {}ms cannot hear the answer to a restart the daemon \
             budgets {SPAWN_READY_TIMEOUT_MS}ms for",
            agent_control_io_timeout_ms(),
        );
    }

    #[test]
    fn an_absent_timeout_uses_the_agents_own_default() {
        assert_eq!(verb_timeout_ms(&Map::new()), DEFAULT_VERB_TIMEOUT_MS);
        // Zero is not a deadline; it would make every verb time out instantly.
        let mut zero = Map::new();
        zero.insert("timeout_ms".to_string(), Value::from(0u64));
        assert_eq!(verb_timeout_ms(&zero), DEFAULT_VERB_TIMEOUT_MS);
    }

    #[test]
    fn flag_params_coerce_only_the_numeric_keys() {
        let args: Vec<String> = ["--session", "a", "--width", "800", "--url", "1"]
            .iter()
            .map(|value| value.to_string())
            .collect();
        let params = params_from_flags(&args).expect("valid flags");
        assert_eq!(params.get("session"), Some(&Value::String("a".into())));
        assert_eq!(params.get("width").and_then(Value::as_u64), Some(800));
        assert_eq!(
            params.get("url"),
            Some(&Value::String("1".into())),
            "a numeric-LOOKING value for a string key must stay a string, or a url/selector \
             that happens to be digits changes type on the wire",
        );
    }

    #[test]
    fn flag_params_spell_kebab_keys_the_way_the_protocol_does() {
        let args: Vec<String> = ["--timeout-ms", "500"]
            .iter()
            .map(|value| value.to_string())
            .collect();
        let params = params_from_flags(&args).expect("valid flags");
        assert_eq!(
            params.get("timeout_ms").and_then(Value::as_u64),
            Some(500),
            "--timeout-ms is the CLI spelling of the protocol's timeout_ms",
        );
    }

    #[test]
    fn a_dangling_flag_is_refused_rather_than_dropped() {
        let args = vec!["--selector".to_string()];
        assert!(params_from_flags(&args).is_err());
    }

    #[test]
    fn every_failure_arm_names_itself_in_its_summary() {
        // The summary is what a CLI user reads; an arm that summarises to
        // something generic ("failed") makes the typing pointless.
        let arms = [
            WpeOutcome::VerbFailed {
                message: "boom".into(),
            },
            WpeOutcome::NotProvisioned {
                searched: "x".into(),
                detail: "no binary".into(),
            },
            WpeOutcome::StartFailed {
                exit: "exited 1".into(),
                detail: "bring-up failed".into(),
            },
            WpeOutcome::AgentDead {
                pid: 42,
                exit: "killed by signal 9".into(),
            },
            WpeOutcome::Timeout {
                verb: "eval".into(),
                waited_ms: 1_500,
            },
            WpeOutcome::Transport {
                message: "socket gone".into(),
            },
        ];
        for arm in arms {
            assert!(!arm.is_answer());
            let summary = arm.summary();
            assert!(
                summary.len() > 6 && summary != "failed",
                "{arm:?} summarises to {summary:?}, which tells the caller nothing",
            );
        }
    }
}
